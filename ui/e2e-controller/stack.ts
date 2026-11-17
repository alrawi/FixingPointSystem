// SPDX-License-Identifier: GPL-3.0-only

// Starts the real stack that the browser tests talk to: the fps-controller
// binary on free ports with a temporary data directory, the fps-fakeagent
// helper with two fake agents, the example modules, and the built UI behind
// `vite preview`, which forwards /v1 to the controller. Nothing here is a mock.
//
// The Go binaries come from FPS_CONTROLLER_BIN and FPS_FAKEAGENT_BIN when those
// are set, and otherwise from `go build` in ../controller, which needs the Go
// module proxy. A machine that cannot build them fails the suite with the
// build output instead of skipping it, because a skipped suite proves nothing.

import { execFile, spawn, type ChildProcess } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { promisify } from "node:util";
import {
  ADMIN_TOKEN,
  AGENT_LOCAL,
  AGENT_SPARE,
  CONTROLLER_URL_VAR,
  FINISH_AFTER_SECONDS,
  PREVIEW_PORT,
  READ_TOKEN,
  RUN_TOKEN,
} from "./constants";

const run = promisify(execFile);

const uiDir = resolve(import.meta.dirname, "..");
const repoDir = resolve(uiDir, "..");
const controllerDir = join(repoDir, "controller");
const examplesDir = join(repoDir, "contracts", "examples");

/** A helper process with the text that it wrote, kept for the log files. */
interface Managed {
  name: string;
  child: ChildProcess;
  stdout: string;
  stderr: string;
  exited: boolean;
}

const MAX_LOG = 2_000_000;

function launch(name: string, command: string, args: string[], env?: NodeJS.ProcessEnv): Managed {
  const child = spawn(command, args, {
    env: { ...process.env, ...env },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const managed: Managed = { name, child, stdout: "", stderr: "", exited: false };
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    managed.stdout = (managed.stdout + chunk).slice(-MAX_LOG);
  });
  child.stderr.on("data", (chunk: string) => {
    managed.stderr = (managed.stderr + chunk).slice(-MAX_LOG);
  });
  child.on("exit", () => {
    managed.exited = true;
  });
  return managed;
}

async function waitFor<T>(
  what: string,
  timeoutMs: number,
  check: () => T | undefined | Promise<T | undefined>,
  failed?: () => string | undefined,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await check();
    if (value !== undefined) return value;
    const problem = failed?.();
    if (problem !== undefined) throw new Error(`${what}: ${problem}`);
    if (Date.now() > deadline) throw new Error(`${what}: not ready after ${String(timeoutMs)} ms`);
    await new Promise((done) => setTimeout(done, 100));
  }
}

/** The addr of the first JSON log record with the message, which the controller writes to stderr. */
function logAddr(managed: Managed, message: string): string | undefined {
  for (const line of managed.stderr.split("\n")) {
    if (!line.startsWith("{")) continue;
    try {
      const record = JSON.parse(line) as { msg?: string; addr?: string };
      if (record.msg === message && typeof record.addr === "string") return record.addr;
    } catch {
      // A line that is not a complete JSON record is not the one that we wait for.
    }
  }
  return undefined;
}

async function buildGo(outDir: string, envName: string, file: string, pkg: string) {
  const given = process.env[envName];
  if (given !== undefined && given !== "") return resolve(given);
  const out = join(outDir, file);
  try {
    await run("go", ["build", "-o", out, pkg], {
      cwd: controllerDir,
      env: { ...process.env, GOWORK: "off" },
      timeout: 600_000,
      maxBuffer: 10_000_000,
    });
  } catch (error) {
    const detail = error as { stdout?: string; stderr?: string; message: string };
    throw new Error(
      `go build ${pkg} failed, so the suite has no controller to test against. ` +
        `Set ${envName} to a built binary, or run it where the Go modules download.\n` +
        `${detail.stderr ?? ""}${detail.stdout ?? ""}${detail.message}`,
      { cause: error },
    );
  }
  return out;
}

/** A gzip compressed tar archive of a module directory, as POST /v1/modules takes it. */
async function moduleArchive(dir: string, outFile: string) {
  await run("tar", ["-czf", outFile, "-C", dir, ...readdirSync(dir)]);
}

async function rest(
  controllerUrl: string,
  method: string,
  path: string,
  contentType: string,
  body: string | Blob,
  expect: number,
) {
  const response = await fetch(`${controllerUrl}${path}`, {
    method,
    headers: { Authorization: `Bearer ${ADMIN_TOKEN}`, "Content-Type": contentType },
    body,
  });
  if (response.status !== expect) {
    throw new Error(
      `${method} ${path}: status ${String(response.status)}, want ${String(expect)}: ${await response.text()}`,
    );
  }
}

/** A running stack. `stop` ends every process and writes their logs. */
export interface Stack {
  controllerUrl: string;
  previewUrl: string;
  stop: () => Promise<void>;
}

export async function startStack(): Promise<Stack> {
  if (!existsSync(join(uiDir, "dist", "index.html"))) {
    throw new Error("ui/dist is missing. Run `npm run build` first.");
  }
  const work = mkdtempSync(join(tmpdir(), "fps-ui-e2e-"));
  const processes: Managed[] = [];
  const logDir = join(uiDir, "test-results", "stack");

  async function stop() {
    for (const managed of processes.toReversed()) {
      if (!managed.exited) managed.child.kill("SIGTERM");
    }
    await new Promise((done) => setTimeout(done, 500));
    for (const managed of processes) {
      if (!managed.exited) managed.child.kill("SIGKILL");
    }
    mkdirSync(logDir, { recursive: true });
    for (const managed of processes) {
      writeFileSync(join(logDir, `${managed.name}.stdout.log`), managed.stdout);
      writeFileSync(join(logDir, `${managed.name}.stderr.log`), managed.stderr);
    }
    rmSync(work, { recursive: true, force: true });
  }

  try {
    const controllerBin = await buildGo(
      work,
      "FPS_CONTROLLER_BIN",
      "fps-controller",
      "./cmd/fps-controller",
    );
    const fakeAgentBin = await buildGo(
      work,
      "FPS_FAKEAGENT_BIN",
      "fps-fakeagent",
      "./cmd/fps-fakeagent",
    );

    const configFile = join(work, "controller.yaml");
    writeFileSync(
      configFile,
      [
        "listen: 127.0.0.1:0",
        "grpc_listen: 127.0.0.1:0",
        "insecure_plain_grpc: true",
        "log_level: debug",
        "shutdown_timeout: 10s",
        `data_dir: ${join(work, "data")}`,
        "tokens:",
        `  - { name: admin, scope: admin, token: ${ADMIN_TOKEN} }`,
        `  - { name: runner, scope: run, token: ${RUN_TOKEN} }`,
        `  - { name: viewer, scope: read, token: ${READ_TOKEN} }`,
        "",
      ].join("\n"),
      { mode: 0o600 },
    );
    const controller = launch("controller", controllerBin, ["-config", configFile]);
    processes.push(controller);
    const exitedEarly = (managed: Managed) => () =>
      managed.exited ? `${managed.name} exited early:\n${managed.stderr}` : undefined;
    const restAddr = await waitFor(
      "controller listening",
      30_000,
      () => logAddr(controller, "controller listening"),
      exitedEarly(controller),
    );
    const grpcAddr = await waitFor(
      "agent service listening",
      30_000,
      () => logAddr(controller, "agent service listening"),
      exitedEarly(controller),
    );
    const controllerUrl = `http://${restAddr}`;

    // Enroll the agents and install the modules that the example configurations use.
    for (const id of [AGENT_LOCAL, AGENT_SPARE]) {
      await rest(
        controllerUrl,
        "POST",
        "/v1/agents",
        "application/json",
        JSON.stringify({ id }),
        201,
      );
    }
    for (const name of ["http1", "dns"]) {
      const archive = join(work, `${name}.tar.gz`);
      await moduleArchive(join(examplesDir, "modules", name), archive);
      await rest(
        controllerUrl,
        "POST",
        "/v1/modules",
        "application/gzip",
        new Blob([readFileSync(archive)]),
        201,
      );
    }

    const agents = launch("fakeagent", fakeAgentBin, [
      "-controller",
      grpcAddr,
      "-agents",
      `${AGENT_LOCAL},${AGENT_SPARE}`,
      "-ports",
      "lo",
      "-finish-after",
      `${String(FINISH_AFTER_SECONDS)}s`,
    ]);
    processes.push(agents);
    await waitFor(
      "fake agents ready",
      30_000,
      () => (agents.stdout.includes("fake agents ready") ? true : undefined),
      exitedEarly(agents),
    );

    const preview = launch(
      "preview",
      process.execPath,
      [
        join(uiDir, "node_modules", "vite", "bin", "vite.js"),
        "preview",
        "--host",
        "127.0.0.1",
        "--port",
        String(PREVIEW_PORT),
        "--strictPort",
      ],
      { FPS_API_TARGET: controllerUrl },
    );
    processes.push(preview);
    const previewUrl = `http://127.0.0.1:${String(PREVIEW_PORT)}`;
    await waitFor(
      "preview server",
      30_000,
      async () => {
        try {
          return (await fetch(previewUrl)).ok ? true : undefined;
        } catch {
          return undefined;
        }
      },
      exitedEarly(preview),
    );
    // The proxy must reach the real controller: a token check proves the path.
    const probe = await fetch(`${previewUrl}/v1/capabilities`, {
      headers: { Authorization: `Bearer ${RUN_TOKEN}` },
    });
    if (!probe.ok)
      throw new Error(`GET /v1/capabilities through the preview: ${String(probe.status)}`);

    process.env[CONTROLLER_URL_VAR] = controllerUrl;
    return { controllerUrl, previewUrl, stop };
  } catch (error) {
    for (const managed of processes) {
      process.stderr.write(`--- ${managed.name} stderr ---\n${managed.stderr.slice(-20_000)}\n`);
    }
    await stop();
    throw error;
  }
}
