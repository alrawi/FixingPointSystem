// SPDX-License-Identifier: GPL-3.0-only

import AxeBuilder from "@axe-core/playwright";
import { expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { AGENT_LOCAL, CONTROLLER_URL_VAR, RUN_TOKEN } from "./constants";

const smokePath = resolve(
  import.meta.dirname,
  "..",
  "..",
  "contracts",
  "examples",
  "tests",
  "http_smoke.yaml",
);

/** A real browser computes layout, so the scan includes the color contrast rule. */
export async function expectAccessible(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
}

export async function signIn(page: Page, token = RUN_TOKEN) {
  await page.goto("/");
  await page.getByLabel("API token").fill(token);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Capabilities" })).toBeVisible();
}

export async function openFromNavigation(page: Page, name: string) {
  await page.getByRole("navigation", { name: "Main" }).getByRole("link", { name }).click();
}

function controllerUrl(): string {
  const url = process.env[CONTROLLER_URL_VAR];
  if (url === undefined) throw new Error(`${CONTROLLER_URL_VAR} is not set by the global setup`);
  return url;
}

/** One call of the REST API of the real controller, with the run token. */
export async function rest<T>(
  method: string,
  path: string,
  options: { body?: string; contentType?: string; expect?: number } = {},
): Promise<T> {
  const response = await fetch(`${controllerUrl()}${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${RUN_TOKEN}`,
      ...(options.contentType === undefined ? {} : { "Content-Type": options.contentType }),
    },
    ...(options.body === undefined ? {} : { body: options.body }),
  });
  const want = options.expect ?? 200;
  const text = await response.text();
  if (response.status !== want) {
    throw new Error(
      `${method} ${path}: status ${String(response.status)}, want ${String(want)}: ${text}`,
    );
  }
  return (text === "" ? undefined : JSON.parse(text)) as T;
}

interface AgentState {
  id: string;
  state: string;
}

interface RunState {
  id: string;
  state: string;
  verdict?: { passed?: boolean };
}

const FINAL = new Set(["completed", "failed", "aborted"]);

async function sleep(ms: number) {
  await new Promise((done) => setTimeout(done, ms));
}

/** Waits until the agent is online again, which is when it can take a run. */
export async function waitForAgentOnline(id = AGENT_LOCAL, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const agents = await rest<AgentState[]>("GET", "/v1/agents");
    if (agents.some((agent) => agent.id === id && agent.state === "online")) return;
    if (Date.now() > deadline) {
      throw new Error(`agent ${id} is not online: ${JSON.stringify(agents)}`);
    }
    await sleep(250);
  }
}

/** The example configuration `http_smoke` under another name, with the given criteria block. */
export function smokeConfig(name: string, criteria?: string): string {
  let text = readFileSync(smokePath, "utf8").replace("  name: http_smoke\n", `  name: ${name}\n`);
  if (criteria !== undefined) {
    const at = text.indexOf("\ncriteria:");
    if (at < 0) throw new Error("http_smoke.yaml has no criteria block");
    text = `${text.slice(0, at + 1)}criteria:\n${criteria}\n`;
  }
  return text;
}

/** Stores a configuration through the REST API and returns its id. */
export async function createConfig(yaml: string): Promise<string> {
  const ref = await rest<{ id: string }>("POST", "/v1/configs", {
    body: yaml,
    contentType: "application/yaml",
    expect: 201,
  });
  return ref.id;
}

/** Starts a run on the agent `local` and returns it as the controller answers. */
export async function startRun(configId: string): Promise<RunState> {
  await waitForAgentOnline();
  return rest<RunState>("POST", "/v1/runs", {
    body: JSON.stringify({ config_id: configId, agents: [AGENT_LOCAL] }),
    contentType: "application/json",
    expect: 202,
  });
}

/** Polls the run until it reaches a final state. The fake agents end a plan after ten seconds. */
export async function waitForFinalRun(id: string, timeoutMs = 60_000): Promise<RunState> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const run = await rest<RunState>("GET", `/v1/runs/${id}`);
    if (FINAL.has(run.state)) return run;
    if (Date.now() > deadline) throw new Error(`run ${id} did not end: ${JSON.stringify(run)}`);
    await sleep(500);
  }
}

/** Stores a configuration, runs it to its end on the fake agent, and returns the id of the run. */
export async function runToEnd(yaml: string): Promise<{ configId: string; run: RunState }> {
  const configId = await createConfig(yaml);
  const started = await startRun(configId);
  const run = await waitForFinalRun(started.id);
  return { configId, run };
}
