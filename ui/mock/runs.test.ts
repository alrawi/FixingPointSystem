// SPDX-License-Identifier: GPL-3.0-only
//
// Contract tests of the run and metrics operations of the mock server: every
// response validates against the response schema of its operation in
// fps-api-v1.yaml, and the event stream carries the event names and the data
// of the contract.
// @vitest-environment node

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { Ajv2020 } from "ajv/dist/2020.js";
import addFormatsModule from "ajv-formats";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { parse } from "yaml";
import { EventStreamParser, type ServerEvent } from "../src/api/eventStream";
import { CONTRACTS_DIR } from "./fixtures";
import { MOCK_RUN_EPOCH_MS, scenarioTotalsAt } from "./runs";
import { MOCK_TOKEN, startMockServer, type RunningMockServer } from "./server";

const API_ID = "https://github.com/alrawi/FixingPointSystem/contracts/openapi/fps-api-v1.yaml";

const addFormats = ((addFormatsModule as unknown as { default?: unknown }).default ??
  addFormatsModule) as unknown as (ajv: Ajv2020) => void;

function buildValidator(): Ajv2020 {
  const ajv = new Ajv2020({ strict: false, validateSchema: false, allErrors: true });
  addFormats(ajv);
  const schemaDir = join(CONTRACTS_DIR, "schemas");
  for (const file of readdirSync(schemaDir).filter((name) => name.endsWith(".json"))) {
    ajv.addSchema(JSON.parse(readFileSync(join(schemaDir, file), "utf8")) as object);
  }
  const api = parse(readFileSync(join(CONTRACTS_DIR, "openapi", "fps-api-v1.yaml"), "utf8")) as {
    [key: string]: unknown;
  };
  ajv.addSchema({ ...api, $id: API_ID });
  return ajv;
}

function pointer(...parts: string[]): string {
  return parts.map((part) => part.replaceAll("~", "~0").replaceAll("/", "~1")).join("/");
}

function responseSchema(path: string, method: string, status: string): string {
  return pointer(
    "paths",
    path,
    method,
    "responses",
    status,
    "content",
    "application/json",
    "schema",
  );
}

const ajv = buildValidator();

function expectValid(ref: string, body: unknown) {
  const validate = ajv.compile({ $ref: `${API_ID}#/${ref}` });
  expect(validate(body), JSON.stringify(validate.errors, null, 2)).toBe(true);
}

/** A clock that tests move by hand, in ticks of one second. */
class Clock {
  ms = 0;
  tick(count = 1): void {
    this.ms += count * 1000;
  }
  now = (): number => this.ms;
}

const headers = { Authorization: `Bearer ${MOCK_TOKEN}`, "Content-Type": "application/json" };

describe("mock runs", () => {
  let mock: RunningMockServer;
  let clock: Clock;

  beforeEach(async () => {
    clock = new Clock();
    mock = await startMockServer(0, "127.0.0.1", { runs: { now: clock.now, runTicks: 10 } });
  });
  afterEach(async () => {
    await mock.close();
  });

  async function start(body: unknown = { config_id: "cfg_http_smoke" }) {
    return fetch(`${mock.baseUrl}/runs`, { method: "POST", headers, body: JSON.stringify(body) });
  }

  async function getRun(id: string) {
    const response = await fetch(`${mock.baseUrl}/runs/${id}`, { headers });
    expect(response.status).toBe(200);
    const run = (await response.json()) as {
      state: string;
      summary?: { sessions_attempted: number };
      reason?: string;
      ended_at?: string;
    };
    expectValid(responseSchema("/runs/{id}", "get", "200"), run);
    return run;
  }

  it("starts a run with 202 and a Location, then walks it to completed", async () => {
    const response = await start();
    expect(response.status).toBe(202);
    const run = (await response.json()) as { id: string; state: string; config_id: string };
    expectValid(responseSchema("/runs", "post", "202"), run);
    expect(response.headers.get("Location")).toBe(`/v1/runs/${run.id}`);
    expect(run).toMatchObject({ config_id: "cfg_http_smoke", state: "pending" });

    const seen: string[] = [run.state];
    for (let step = 0; step < 12; step += 1) {
      clock.tick();
      seen.push((await getRun(run.id)).state);
    }
    expect([...new Set(seen)]).toEqual(["pending", "preparing", "running", "completed"]);
    const done = await getRun(run.id);
    expect(done.summary?.sessions_attempted).toBe(800);
    expect(done.ended_at).toBeDefined();
  });

  it("lists runs newest first, filters by state, and pages", async () => {
    await start();
    clock.tick(3);
    await start();
    const list = await fetch(`${mock.baseUrl}/runs`, { headers });
    const body = (await list.json()) as { items: { id: string; state: string }[] };
    expectValid(responseSchema("/runs", "get", "200"), body);
    expect(body.items.map((run) => run.id)).toEqual(["run_mock0002", "run_mock0001"]);
    const running = await fetch(`${mock.baseUrl}/runs?state=running`, { headers });
    const filtered = (await running.json()) as { items: { id: string }[] };
    expect(filtered.items.map((run) => run.id)).toEqual(["run_mock0001"]);
    const paged = await fetch(`${mock.baseUrl}/runs?page_size=1`, { headers });
    const page = (await paged.json()) as { items: unknown[]; next_page_token?: string };
    expect(page.items).toHaveLength(1);
    expect(page.next_page_token).toBeDefined();
    const bad = await fetch(`${mock.baseUrl}/runs?state=sleeping`, { headers });
    expect(bad.status).toBe(400);
  });

  it("stops a run through stopping to aborted and refuses a second stop after the end", async () => {
    const { id } = (await (await start()).json()) as { id: string };
    clock.tick(4);
    const stop = await fetch(`${mock.baseUrl}/runs/${id}:stop`, { method: "POST", headers });
    expect(stop.status).toBe(202);
    const stopping = (await stop.json()) as { state: string };
    expectValid(responseSchema("/runs/{id}:stop", "post", "202"), stopping);
    expect(stopping.state).toBe("stopping");
    clock.tick();
    const aborted = await getRun(id);
    expect(aborted.state).toBe("aborted");
    expect(aborted.reason).toBe("stopped by request");
    const again = await fetch(`${mock.baseUrl}/runs/${id}:stop`, { method: "POST", headers });
    expect(again.status).toBe(409);
    expectValid("components/schemas/Problem", await again.json());
  });

  it("aborts at once on a forced stop", async () => {
    const { id } = (await (await start()).json()) as { id: string };
    clock.tick(4);
    const stop = await fetch(`${mock.baseUrl}/runs/${id}:stop`, {
      method: "POST",
      headers,
      body: JSON.stringify({ force: true }),
    });
    expect(((await stop.json()) as { state: string }).state).toBe("aborted");
  });

  it("answers problems for unknown configurations, agents, runs, and bad requests", async () => {
    for (const [response, status] of [
      [await start({ config_id: "cfg_nothing" }), 404],
      [await start({ config_id: "cfg_http_smoke", agents: ["ghost"] }), 404],
      [await start({}), 400],
      [await fetch(`${mock.baseUrl}/runs/run_none`, { headers }), 404],
      [await fetch(`${mock.baseUrl}/runs/run_none/metrics?view=application`, { headers }), 404],
    ] as const) {
      expect(response.status).toBe(status);
      expectValid("components/schemas/Problem", await response.json());
    }
  });

  it("accepts known agents", async () => {
    const response = await start({ config_id: "cfg_http_smoke", agents: ["agent-local"] });
    expect(response.status).toBe(202);
  });

  it("serves the metric series of a view, from a time on", async () => {
    const { id } = (await (await start()).json()) as { id: string };
    clock.tick(6);
    const all = await fetch(`${mock.baseUrl}/runs/${id}/metrics?view=application`, { headers });
    const set = (await all.json()) as { view: string; samples: { ts: string }[] };
    expectValid(responseSchema("/runs/{id}/metrics", "get", "200"), set);
    expect(set.view).toBe("application");
    expect(set.samples.map((sample) => sample.ts)).toEqual(
      [1, 2, 3, 4, 5].map((tick) => new Date(MOCK_RUN_EPOCH_MS + tick * 1000).toISOString()),
    );
    const from = new Date(MOCK_RUN_EPOCH_MS + 4000).toISOString();
    const later = await fetch(
      `${mock.baseUrl}/runs/${id}/metrics?view=application&from=${encodeURIComponent(from)}`,
      { headers },
    );
    expect(((await later.json()) as { samples: unknown[] }).samples).toHaveLength(2);
    const port = await fetch(`${mock.baseUrl}/runs/${id}/metrics?view=port`, { headers });
    expect(((await port.json()) as { samples: unknown[] }).samples).toEqual([]);
    const missing = await fetch(`${mock.baseUrl}/runs/${id}/metrics`, { headers });
    expect(missing.status).toBe(400);
  });

  /** Reads a stream to its end and returns the events and the raw text. */
  async function readStream(id: string, query = "view=application") {
    const response = await fetch(`${mock.baseUrl}/runs/${id}/metrics:stream?${query}`, {
      headers: { Authorization: `Bearer ${MOCK_TOKEN}`, Accept: "text/event-stream" },
    });
    expect(response.status).toBe(200);
    expect(response.headers.get("Content-Type")).toBe("text/event-stream");
    const parser = new EventStreamParser();
    const events: ServerEvent[] = [];
    events.push(...parser.push(await response.text()));
    return { events, parser };
  }

  it("streams sample events with the contract data and closes with an end event", async () => {
    const { id } = (await (await start()).json()) as { id: string };
    clock.tick(30);
    const { events, parser } = await readStream(id);
    expect(parser.retryMs).toBe(1000);
    expect(events.map((event) => event.type)).toEqual(["message", "end"]);
    const sample = JSON.parse(events[0]?.data ?? "") as { view: string; key: string };
    expectValid("components/schemas/MetricSample", sample);
    expect(sample).toMatchObject({ view: "application", key: "http1" });
    expect(events[0]?.lastEventId).toBe("1");
    expect(JSON.parse(events[1]?.data ?? "")).toEqual({ state: "completed" });
  });

  it("sends the views that the request names, and all views when it names none", async () => {
    const { id } = (await (await start()).json()) as { id: string };
    clock.tick(30);
    const both = await readStream(id, "view=application&view=scenario&side=client");
    expect(
      both.events.filter((event) => event.type === "message").map((event) => event.data),
    ).toHaveLength(2);
    const all = await readStream(id, "");
    const views = all.events
      .filter((event) => event.type === "message")
      .map((event) => (JSON.parse(event.data) as { view: string }).view);
    expect(views.sort()).toEqual(["application", "scenario"]);
  });

  it("ends the stream of a stopped run with the aborted state", async () => {
    const { id } = (await (await start()).json()) as { id: string };
    clock.tick(4);
    await fetch(`${mock.baseUrl}/runs/${id}:stop`, {
      method: "POST",
      headers,
      body: JSON.stringify({ force: true }),
    });
    const { events } = await readStream(id);
    expect(JSON.parse(events[events.length - 1]?.data ?? "")).toEqual({ state: "aborted" });
  });
});

describe("mock stream drops", () => {
  it("cuts the first connections without an end event, then serves the end", async () => {
    const mock = await startMockServer(0, "127.0.0.1", {
      runs: { stepMs: 20, runTicks: 40 },
      streamDrops: 1,
    });
    try {
      const { id } = (await (
        await fetch(`${mock.baseUrl}/runs`, {
          method: "POST",
          headers,
          body: JSON.stringify({ config_id: "cfg_http_smoke" }),
        })
      ).json()) as { id: string };
      const url = `${mock.baseUrl}/runs/${id}/metrics:stream?view=application`;
      const first = await fetch(url, { headers });
      let text = "";
      let failed = false;
      try {
        text = await first.text();
      } catch {
        failed = true;
      }
      expect(failed || !text.includes("event: end")).toBe(true);
    } finally {
      await mock.close();
    }
  });
});

describe("mock reports", () => {
  let mock: RunningMockServer;
  let clock: Clock;

  beforeEach(async () => {
    clock = new Clock();
    mock = await startMockServer(0, "127.0.0.1", {
      runs: {
        now: clock.now,
        runTicks: 10,
        verdictStates: true,
        failedEvery: (ordinal) => (ordinal === 1 ? 0 : 50),
      },
    });
  });
  afterEach(async () => {
    await mock.close();
  });

  async function start(configId: string): Promise<string> {
    const response = await fetch(`${mock.baseUrl}/runs`, {
      method: "POST",
      headers,
      body: JSON.stringify({ config_id: configId }),
    });
    return ((await response.json()) as { id: string }).id;
  }

  async function report(id: string) {
    const response = await fetch(`${mock.baseUrl}/runs/${id}/report`, { headers });
    expect(response.status).toBe(200);
    expect(response.headers.get("Content-Type")).toBe("application/json");
    const body = (await response.json()) as {
      run: { state: string; reason?: string; verdict?: { passed?: boolean; criteria?: unknown[] } };
      realism: { property: string; passed: boolean }[];
    };
    expectValid(responseSchema("/runs/{id}/report", "get", "200"), body);
    return body;
  }

  it("serves a passing report that validates against the contract", async () => {
    const id = await start("cfg_http_smoke");
    clock.tick(12);
    const body = await report(id);
    expect(body.run.state).toBe("completed");
    expect(body.run.verdict).toEqual({
      passed: true,
      criteria: [
        {
          metric: "sessions.failed_ratio",
          op: "==",
          value: 0,
          observed: 0,
          passed: true,
          reason: "observed 0 == 0 holds",
        },
      ],
    });
    expect(body.realism.map((result) => result.passed)).toEqual([true, true, true]);
  });

  it("serves the counts of each scenario, which add up to the run summary", async () => {
    await start("cfg_http_smoke");
    const id = await start("cfg_filtered_criteria");
    clock.tick(12);
    const response = await fetch(`${mock.baseUrl}/runs/${id}/report`, { headers });
    const body = (await response.json()) as {
      run: { summary: Record<string, number> };
      scenarios: Record<string, string | number>[];
    };
    expectValid(responseSchema("/runs/{id}/report", "get", "200"), body);
    expect(body.scenarios).toEqual([
      {
        application: "dns",
        scenario: "lookup",
        sessions_attempted: 80,
        sessions_succeeded: 74,
        sessions_failed: 1,
        bytes_tx: 163840,
        bytes_rx: 655360,
      },
      {
        application: "http1",
        scenario: "web_get",
        sessions_attempted: 720,
        sessions_succeeded: 661,
        sessions_failed: 14,
        bytes_tx: 1474560,
        bytes_rx: 5898240,
      },
    ]);
    expect(body.run.summary["sessions_attempted"]).toBe(800);
    expect(body.run.summary["bytes_rx"]).toBe(6553600);
  });

  it("sends an empty scenario list before the first tick", async () => {
    const id = await start("cfg_http_smoke");
    const body = await report(id);
    expect((body as { scenarios?: unknown }).scenarios).toEqual([]);
  });

  it("divides the counts of an application among its scenarios by weight", () => {
    const app = {
      key: "http1",
      scenario: "http1/web_get",
      scenarios: [
        { name: "web_post", weight: 1 },
        { name: "web_get", weight: 2 },
      ],
      rate: 90,
      failedEvery: 0,
    };
    // At tick 10 the application attempted 720 sessions and holds 45 concurrent ones.
    expect(scenarioTotalsAt([app], 10).map((s) => [s.scenario, s.sessions_attempted])).toEqual([
      ["web_get", 480],
      ["web_post", 240],
    ]);
    expect(scenarioTotalsAt([app], 0)).toEqual([]);
  });

  it("serves a failing report with the reason of the controller", async () => {
    await start("cfg_http_smoke");
    const id = await start("cfg_http_smoke");
    clock.tick(12);
    const body = await report(id);
    expect(body.run.state).toBe("failed");
    expect(body.run.reason).toBe(
      "criteria failed: sessions.failed_ratio: observed 0.02 == 0 does not hold",
    );
    expect(body.run.verdict?.passed).toBe(false);
    expect(body.realism.map((result) => result.passed)).toEqual([true, true, false]);
  });

  it("leaves out observed and passed for a metric without a value", async () => {
    const id = await start("cfg_enterprise_mix_10g");
    clock.tick(12);
    const body = await report(id);
    expect(body.run.verdict?.passed).toBe(true);
    expect(body.run.verdict?.criteria?.[1]).toEqual({
      metric: "mix.max_abs_error_pp",
      op: "<=",
      value: 2,
      reason: "metric mix.max_abs_error_pp has no value",
    });
  });

  it("serves a report with no verdict while the run is active and after an abort", async () => {
    const id = await start("cfg_http_smoke");
    clock.tick(4);
    const active = await report(id);
    expect(active.run.verdict).toBeUndefined();
    expect(active.realism).toEqual([]);
    await fetch(`${mock.baseUrl}/runs/${id}:stop`, {
      method: "POST",
      headers,
      body: JSON.stringify({ force: true }),
    });
    const aborted = await report(id);
    expect(aborted.run.state).toBe("aborted");
    expect(aborted.run.verdict).toBeUndefined();
  });

  it("answers 404 for a missing run and 501 for the formats that it does not serve", async () => {
    const missing = await fetch(`${mock.baseUrl}/runs/run_none/report`, { headers });
    expect(missing.status).toBe(404);
    const id = await start("cfg_http_smoke");
    const csv = await fetch(`${mock.baseUrl}/runs/${id}/report?format=csv`, { headers });
    expect(csv.status).toBe(501);
  });

  it("keeps the scripted state without the verdictStates option", async () => {
    const plain = await startMockServer(0, "127.0.0.1", {
      runs: { now: clock.now, runTicks: 10 },
    });
    try {
      const { id } = (await (
        await fetch(`${plain.baseUrl}/runs`, {
          method: "POST",
          headers,
          body: JSON.stringify({ config_id: "cfg_http_smoke" }),
        })
      ).json()) as { id: string };
      clock.tick(12);
      const body = (await (
        await fetch(`${plain.baseUrl}/runs/${id}/report`, { headers })
      ).json()) as { run: { state: string; verdict?: { passed?: boolean } } };
      expect(body.run.state).toBe("completed");
      expect(body.run.verdict?.passed).toBe(false);
    } finally {
      await plain.close();
    }
  });
});
