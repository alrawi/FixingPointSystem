// SPDX-License-Identifier: GPL-3.0-only
//
// A mock of the controller REST API for UI tests and local development. It
// serves the read operations that the UI uses so far from the contract
// examples, checks the bearer token, pages module lists, injects configured
// faults, and answers everything else with an RFC 9457 problem. It needs only
// node:http, so it adds no dependency.

import { readFileSync } from "node:fs";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { join } from "node:path";
import { Ajv2020 } from "ajv/dist/2020.js";
import { parse } from "yaml";
import type { Agent, Finding, Problem, ValidationReport } from "../src/api/client";
import { CONTRACTS_DIR, loadMockData, type MockData, type StoredConfig } from "./fixtures";
import { MockRuns, unknownAgents, type MockRunOptions } from "./runs";

/** The token that the mock server accepts unless the options name others. */
export const MOCK_TOKEN = "fps-mock-token";

/** The holder that reserveAgent records, as the controller does. */
const OPERATOR = "operator";

/** An error answer that replaces the normal answer of matching requests, for error tests. */
export interface MockFault {
  /** The method to fail, every method when absent. */
  method?: string;
  /** The path below /v1, such as `/modules`, or a pattern for the path below /v1. */
  path: string | RegExp;
  status: number;
  /** The detail of the problem body. */
  detail?: string;
}

export interface MockServerOptions {
  /** Tokens that the server accepts. */
  tokens?: string[];
  data?: MockData;
  /** Errors to answer in place of the normal answers, checked after the token. */
  faults?: MockFault[];
  /** The clock, tick length, and run length of the mock runs. */
  runs?: MockRunOptions;
  /**
   * The number of metric stream connections that the server cuts after two ticks without an
   * end event, oldest first, to test reconnects.
   */
  streamDrops?: number;
}

/** A running mock server. */
export interface RunningMockServer {
  server: Server;
  /** The API base URL, ending in /v1. */
  baseUrl: string;
  close: () => Promise<void>;
  /**
   * The live agents of the server. A test changes the `state` of one to simulate an agent that
   * goes offline or starts a run, as another user or the controller would.
   */
  agents: Agent[];
}

/** The body of a request and its content type. */
interface RequestInput {
  body: string;
  contentType: string;
}

type Handler = (
  match: RegExpExecArray,
  url: URL,
  input: RequestInput,
) => { status: number; body: unknown; headers?: Record<string, string> };

function problem(status: number, title: string, detail: string): Problem {
  return { type: "about:blank", title, status, detail };
}

/** The largest page_size that the contract allows. */
const MAX_PAGE_SIZE = 500;
/** The page_size that the contract names as the default. */
const DEFAULT_PAGE_SIZE = 50;

/**
 * Cuts one page out of a list as the PageSize and PageToken parameters
 * describe. The page token is the offset of the first item, which clients
 * treat as opaque.
 */
function paginate(items: unknown[], url: URL): { status: number; body: unknown } {
  const sizeText = url.searchParams.get("page_size");
  const tokenText = url.searchParams.get("page_token");
  const size = sizeText === null ? DEFAULT_PAGE_SIZE : Number(sizeText);
  if (!Number.isInteger(size) || size < 1 || size > MAX_PAGE_SIZE) {
    return {
      status: 400,
      body: problem(400, "Bad Request", `page_size must be an integer from 1 to ${MAX_PAGE_SIZE}.`),
    };
  }
  const offset = tokenText === null ? 0 : Number(tokenText);
  if (!/^\d+$/.test(tokenText ?? "0") || offset > items.length) {
    return { status: 400, body: problem(400, "Bad Request", "The page_token is not valid.") };
  }
  const end = offset + size;
  return {
    status: 200,
    body: {
      items: items.slice(offset, end),
      ...(end < items.length ? { next_page_token: String(end) } : {}),
    },
  };
}

function matchesFault(fault: MockFault, method: string, path: string): boolean {
  if (fault.method !== undefined && fault.method !== method) return false;
  return typeof fault.path === "string" ? fault.path === path : fault.path.test(path);
}

let schemaCheck: ReturnType<Ajv2020["compile"]> | undefined;

/** Checks a body against the test configuration schema, as the controller does on create. */
function schemaFindings(body: unknown): Finding[] {
  schemaCheck ??= new Ajv2020({ allErrors: true, strict: false }).compile(
    JSON.parse(
      readFileSync(join(CONTRACTS_DIR, "schemas", "test-config.schema.json"), "utf8"),
    ) as object,
  );
  if (schemaCheck(body)) return [];
  return (schemaCheck.errors ?? [])
    .filter((error) => !error.schemaPath.includes("/anyOf/"))
    .map((error) => {
      const params = error.params as Record<string, unknown>;
      const extra = params["missingProperty"] ?? params["additionalProperty"];
      return {
        severity: "error" as const,
        path: typeof extra === "string" ? `${error.instancePath}/${extra}` : error.instancePath,
        message: error.message ?? "is not valid",
      };
    });
}

/** The checks beyond the schema: every scenario of the mix must come from an installed module. */
function referenceFindings(body: StoredConfig["body"], data: MockData): Finding[] {
  const known = data.modules.flatMap((module) =>
    (module.summary.scenarios ?? []).map((scenario) => `${module.summary.name}/${scenario}`),
  );
  return body.profile.mix.flatMap((entry, index): Finding[] => {
    const ref = entry.scenario;
    const found = ref.includes("/")
      ? known.includes(ref)
      : known.filter((name) => name.endsWith(`/${ref}`)).length === 1;
    return found
      ? []
      : [
          {
            severity: "error",
            path: `/profile/mix/${String(index)}/scenario`,
            message: `scenario ${ref} is not provided by an installed module`,
          },
        ];
  });
}

/** Reads a configuration body as JSON or YAML, or returns the reason it cannot. */
function decodeBody(input: RequestInput): { value: unknown } | { error: string } {
  const type = input.contentType.split(";")[0]?.trim().toLowerCase() ?? "";
  try {
    if (type === "application/json") return { value: JSON.parse(input.body) as unknown };
    if (type === "application/yaml") return { value: parse(input.body) as unknown };
  } catch (error) {
    return { error: error instanceof Error ? error.message : "The body cannot be parsed." };
  }
  return { error: "Send the configuration as application/json or application/yaml." };
}

/** The application name of a scenario reference such as `http1/web_get`. */
function moduleOf(ref: string, data: MockData): string {
  if (ref.includes("/")) return ref.split("/")[0] ?? ref;
  const found = data.modules.find((module) => (module.summary.scenarios ?? []).includes(ref));
  return found?.summary.name ?? ref;
}

const VIEWS = ["application", "scenario", "port", "tcp", "tls", "realism"] as const;
type View = (typeof VIEWS)[number];

function isView(value: string | null): value is View {
  return VIEWS.some((view) => view === value);
}

const RUN_STATES = [
  "pending",
  "preparing",
  "running",
  "stopping",
  "completed",
  "failed",
  "aborted",
];

function routes(data: MockData, runs: MockRuns, agents: Agent[]): [string, RegExp, Handler][] {
  const configs = [...data.configs];
  let created = 0;
  return [
    [
      "GET",
      /^\/v1\/configs$/,
      (_match, url) =>
        paginate(
          // Newest first, as the contract says.
          [...configs].reverse().map(({ id, name, created_at }) => ({ id, name, created_at })),
          url,
        ),
    ],
    [
      "POST",
      /^\/v1\/configs$/,
      (_match, _url, input) => {
        const decoded = decodeBody(input);
        if ("error" in decoded) {
          const status = decoded.error.startsWith("Send") ? 415 : 400;
          return { status, body: problem(status, "Bad Request", decoded.error) };
        }
        const findings = schemaFindings(decoded.value);
        if (findings.length > 0) {
          return {
            status: 422,
            body: {
              ...problem(
                422,
                "Unprocessable Content",
                `the configuration breaks the schema at ${findings[0]?.path ?? ""}`,
              ),
              errors: findings,
            },
          };
        }
        const body = decoded.value as StoredConfig["body"];
        created += 1;
        const stored: StoredConfig = {
          id: `cfg_mock${String(created).padStart(4, "0")}`,
          name: body.metadata.name,
          created_at: new Date(Date.UTC(2026, 11, 1, 9, created)).toISOString(),
          body,
        };
        configs.push(stored);
        return {
          status: 201,
          body: { id: stored.id, name: stored.name, created_at: stored.created_at },
          headers: { Location: `/v1/configs/${stored.id}` },
        };
      },
    ],
    [
      "GET",
      /^\/v1\/configs\/([^/:]+)$/,
      (match) => {
        const found = configs.find((config) => config.id === decodeURIComponent(match[1] ?? ""));
        return found
          ? { status: 200, body: found.body }
          : { status: 404, body: problem(404, "Not Found", "No configuration has this id.") };
      },
    ],
    [
      "POST",
      /^\/v1\/configs\/([^/:]+):validate$/,
      (match) => {
        const found = configs.find((config) => config.id === decodeURIComponent(match[1] ?? ""));
        if (!found) {
          return { status: 404, body: problem(404, "Not Found", "No configuration has this id.") };
        }
        const findings = [...schemaFindings(found.body), ...referenceFindings(found.body, data)];
        const report: ValidationReport = {
          valid: findings.every((finding) => finding.severity !== "error"),
          findings,
          capacity: { agents_required: 1, agents_available: data.agents.length },
        };
        return { status: 200, body: report };
      },
    ],
    [
      "GET",
      /^\/v1\/capabilities$/,
      () => ({ status: 200, body: { ...data.capabilities, agents } }),
    ],
    [
      "GET",
      /^\/v1\/modules$/,
      (_match, url) => {
        const type = url.searchParams.get("type");
        const items = data.modules
          .map((module) => module.summary)
          .filter((summary) => type === null || summary.type === type);
        return paginate(items, url);
      },
    ],
    [
      "GET",
      /^\/v1\/modules\/([^/]+)\/([^/]+)$/,
      (match) => {
        const name = decodeURIComponent(match[1] ?? "");
        const version = decodeURIComponent(match[2] ?? "");
        const found = data.modules.find(
          (module) => module.summary.name === name && module.summary.version === version,
        );
        return found
          ? { status: 200, body: found.manifest }
          : { status: 404, body: problem(404, "Not Found", "No such module is installed.") };
      },
    ],
    ["GET", /^\/v1\/agents$/, () => ({ status: 200, body: agents })],
    [
      "POST",
      /^\/v1\/agents\/([^/:]+):reserve$/,
      (match) => {
        const id = decodeURIComponent(match[1] ?? "");
        const agent = agents.find((candidate) => candidate.id === id);
        if (!agent) {
          return {
            status: 404,
            body: problem(404, "Not Found", `reserve agent ${id}: unknown agent`),
          };
        }
        // The controller reserves for the operator. It reserves an agent that the operator holds
        // again without error. It refuses in this order: another holder (a run) has the agent,
        // the agent has no control session, the agent holds a plan.
        if (agent.reserved_by !== undefined && agent.reserved_by !== OPERATOR) {
          return {
            status: 409,
            body: problem(
              409,
              "Conflict",
              `reserve agent ${id} for ${OPERATOR}: held by ${agent.reserved_by}: agent reserved by another holder`,
            ),
          };
        }
        if (agent.state === "offline" || agent.state === "busy") {
          const why = agent.state === "offline" ? "" : "plan loaded: ";
          return {
            status: 409,
            body: problem(409, "Conflict", `reserve agent ${id}: ${why}agent ${agent.state}`),
          };
        }
        agent.state = "reserved";
        agent.reserved_by = OPERATOR;
        return { status: 200, body: agent };
      },
    ],
    [
      "POST",
      /^\/v1\/agents\/([^/:]+):release$/,
      (match) => {
        const id = decodeURIComponent(match[1] ?? "");
        const agent = agents.find((candidate) => candidate.id === id);
        if (!agent) {
          return {
            status: 404,
            body: problem(404, "Not Found", `release agent ${id}: unknown agent`),
          };
        }
        // Releasing ends any reservation, whoever holds it, and an agent without one is unchanged.
        // Only the state `reserved` goes back to `online`, because `offline` and `busy` win.
        if (agent.state === "reserved") agent.state = "online";
        delete agent.reserved_by;
        return { status: 200, body: agent };
      },
    ],
    [
      "GET",
      /^\/v1\/runs$/,
      (_match, url) => {
        const state = url.searchParams.get("state");
        if (state !== null && !RUN_STATES.includes(state)) {
          return {
            status: 400,
            body: problem(400, "Bad Request", `state ${state} is not a run state.`),
          };
        }
        return paginate(runs.list(state), url);
      },
    ],
    [
      "POST",
      /^\/v1\/runs$/,
      (_match, _url, input) => {
        const decoded = decodeBody(input);
        if ("error" in decoded) {
          return { status: 400, body: problem(400, "Bad Request", decoded.error) };
        }
        const request = decoded.value as { config_id?: unknown; agents?: unknown };
        if (typeof request.config_id !== "string") {
          return { status: 400, body: problem(400, "Bad Request", "config_id is required.") };
        }
        const agents = Array.isArray(request.agents) ? (request.agents as string[]) : [];
        const config = configs.find((candidate) => candidate.id === request.config_id);
        if (!config) {
          return { status: 404, body: problem(404, "Not Found", "No configuration has this id.") };
        }
        const unknown = unknownAgents(agents, data.agents);
        if (unknown.length > 0) {
          return {
            status: 404,
            body: problem(404, "Not Found", `No agent has the id ${unknown.join(", ")}.`),
          };
        }
        const run = runs.create(config, (ref) => moduleOf(ref, data));
        return { status: 202, body: run, headers: { Location: `/v1/runs/${run.id}` } };
      },
    ],
    [
      "GET",
      /^\/v1\/runs\/([^/:]+)$/,
      (match) => {
        const found = runs.find(decodeURIComponent(match[1] ?? ""));
        return found
          ? { status: 200, body: runs.view(found) }
          : { status: 404, body: problem(404, "Not Found", "No run has this id.") };
      },
    ],
    [
      "POST",
      /^\/v1\/runs\/([^/:]+):stop$/,
      (match, _url, input) => {
        const found = runs.find(decodeURIComponent(match[1] ?? ""));
        if (!found) return { status: 404, body: problem(404, "Not Found", "No run has this id.") };
        const decoded = input.body.trim() === "" ? { value: {} } : decodeBody(input);
        if ("error" in decoded) {
          return { status: 400, body: problem(400, "Bad Request", decoded.error) };
        }
        const force = (decoded.value as { force?: unknown }).force === true;
        const stopped = runs.stop(found, force);
        return stopped
          ? { status: 202, body: stopped }
          : { status: 409, body: problem(409, "Conflict", "The run has ended already.") };
      },
    ],
    [
      "GET",
      /^\/v1\/runs\/([^/:]+)\/report$/,
      (match, url) => {
        const found = runs.find(decodeURIComponent(match[1] ?? ""));
        if (!found) return { status: 404, body: problem(404, "Not Found", "No run has this id.") };
        const format = url.searchParams.get("format") ?? "json";
        if (format !== "json") {
          return {
            status: 501,
            body: problem(
              501,
              "Not Implemented",
              `The mock serves only the json report, not ${format}.`,
            ),
          };
        }
        return { status: 200, body: runs.report(found) };
      },
    ],
    [
      "GET",
      /^\/v1\/runs\/([^/:]+)\/metrics$/,
      (match, url) => {
        const found = runs.find(decodeURIComponent(match[1] ?? ""));
        if (!found) return { status: 404, body: problem(404, "Not Found", "No run has this id.") };
        const view = url.searchParams.get("view");
        if (!isView(view)) {
          return {
            status: 400,
            body: problem(400, "Bad Request", `view must be one of ${VIEWS.join(", ")}.`),
          };
        }
        const from = url.searchParams.get("from");
        if (from !== null && Number.isNaN(Date.parse(from))) {
          return { status: 400, body: problem(400, "Bad Request", "from is not a date-time.") };
        }
        return { status: 200, body: { view, samples: runs.series(found, view, from) } };
      },
    ],
  ];
}

function send(
  res: ServerResponse,
  status: number,
  body: unknown,
  headers: Record<string, string> = {},
): void {
  const isProblem = status >= 400;
  res.writeHead(status, {
    "Content-Type": isProblem ? "application/problem+json" : "application/json",
    "Cache-Control": "no-store",
    ...headers,
  });
  res.end(JSON.stringify(body));
}

/**
 * Answers the metrics stream as the controller does: a retry field, then one event per sample
 * and tick with the connection's tick number as its id, and an `end` event that names the run
 * state after the final samples.
 */
function streamMetrics(
  res: ServerResponse,
  runs: MockRuns,
  run: NonNullable<ReturnType<MockRuns["find"]>>,
  views: View[],
  behavior: { drop: boolean },
): void {
  res.writeHead(200, {
    "Content-Type": "text/event-stream",
    "Cache-Control": "no-store",
  });
  res.write(`retry: ${String(runs.stepMs)}\n\n`);
  let tick = 0;
  const timer = setInterval(() => {
    emit();
  }, runs.stepMs);
  const stop = (): void => {
    clearInterval(timer);
  };
  function emit(): void {
    tick += 1;
    for (const sample of runs.latest(run, views)) {
      res.write(`id: ${String(tick)}\ndata: ${JSON.stringify(sample)}\n\n`);
    }
    const { state, final } = runs.stateOf(run);
    if (final) {
      res.write(`event: end\ndata: ${JSON.stringify({ state })}\n\n`);
      stop();
      res.end();
    } else if (behavior.drop && tick >= 2) {
      stop();
      res.destroy();
    }
  }
  res.on("close", stop);
  emit();
}

/** Creates the request handler of the mock server. */
export function createMockHandler(options: MockServerOptions = {}) {
  const tokens = new Set(options.tokens ?? [MOCK_TOKEN]);
  const data = options.data ?? loadMockData();
  const runs = new MockRuns(options.runs);
  // Reservations change agent states, so the routes work on copies and the data stays as loaded.
  const agents = data.agents.map((agent) => ({ ...agent }));
  const table = routes(data, runs, agents);
  let connections = 0;
  const faults = options.faults ?? [];
  const handle = (req: IncomingMessage, res: ServerResponse, input: RequestInput): void => {
    const url = new URL(req.url ?? "/", "http://mock.invalid");
    const auth = req.headers.authorization ?? "";
    const token = auth.startsWith("Bearer ") ? auth.slice("Bearer ".length) : "";
    if (!tokens.has(token)) {
      res.setHeader("WWW-Authenticate", 'Bearer realm="fps"');
      send(res, 401, problem(401, "Unauthorized", "The bearer token is missing or unknown."));
      return;
    }
    const method = req.method ?? "GET";
    const below = url.pathname.replace(/^\/v1(?=\/)/, "");
    const fault = faults.find((candidate) => matchesFault(candidate, method, below));
    if (fault) {
      const detail = fault.detail ?? `The mock answers ${below} with HTTP ${fault.status}.`;
      send(res, fault.status, problem(fault.status, "Mock Fault", detail));
      return;
    }
    const streamMatch = /^\/v1\/runs\/([^/:]+)\/metrics:stream$/.exec(url.pathname);
    if (streamMatch && method === "GET") {
      const run = runs.find(decodeURIComponent(streamMatch[1] ?? ""));
      if (!run) {
        send(res, 404, problem(404, "Not Found", "No run has this id."));
        return;
      }
      connections += 1;
      const views = url.searchParams.getAll("view").filter(isView);
      streamMetrics(res, runs, run, views.length > 0 ? views : [...VIEWS], {
        drop: connections <= (options.streamDrops ?? 0),
      });
      return;
    }
    let pathMatched = false;
    for (const [routeMethod, pattern, handler] of table) {
      const match = pattern.exec(url.pathname);
      if (!match) continue;
      pathMatched = true;
      if (routeMethod !== method) continue;
      const { status, body, headers } = handler(match, url, input);
      send(res, status, body, headers);
      return;
    }
    if (pathMatched) {
      send(res, 405, problem(405, "Method Not Allowed", "The mock does not serve this method."));
    } else {
      send(res, 404, problem(404, "Not Found", `The mock does not serve ${url.pathname}.`));
    }
  };
  const listener = (req: IncomingMessage, res: ServerResponse): void => {
    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      handle(req, res, {
        body: Buffer.concat(chunks).toString("utf8"),
        contentType: req.headers["content-type"] ?? "",
      });
    });
  };
  return Object.assign(listener, { agents });
}

/** Starts the mock server. Port 0 picks a free port. */
export async function startMockServer(
  port = 0,
  host = "127.0.0.1",
  options: MockServerOptions = {},
): Promise<RunningMockServer> {
  const handler = createMockHandler(options);
  const server = createServer(handler);
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, host, () => {
      resolve();
    });
  });
  const address = server.address() as AddressInfo;
  return {
    server,
    baseUrl: `http://${host}:${address.port}/v1`,
    agents: handler.agents,
    close: () =>
      new Promise<void>((resolve, reject) => {
        server.close((error) => {
          if (error) reject(error);
          else resolve();
        });
      }),
  };
}
