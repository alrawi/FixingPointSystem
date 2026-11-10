// SPDX-License-Identifier: GPL-3.0-only
//
// Component tests of the run list, the run launch, and the live run view
// against the mock server. The acceptance test replays the recorded event
// stream in src/runs/fixtures/run-stream.sse into the view and checks the
// counters and the chart data points that it renders. The expected numbers
// are written out here, not computed from the mock, so a change of the mock
// or of the view shows as a failure.

import { QueryClient } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import axe from "axe-core";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadMockData } from "../../mock/fixtures";
import {
  MOCK_TOKEN,
  startMockServer,
  type MockServerOptions,
  type RunningMockServer,
} from "../../mock/server";
import { App } from "../App";

const FIXTURE = readFileSync(
  join(import.meta.dirname, "..", "runs", "fixtures", "run-stream.sse"),
  "utf8",
);

const running: RunningMockServer[] = [];

afterEach(async () => {
  cleanup();
  await Promise.all(
    running.splice(0).map((mock) => {
      mock.server.closeAllConnections();
      return mock.close();
    }),
  );
});

/** A clock that tests move by hand. One step is one second of run time. */
class Clock {
  ms = 0;
  advance(ticks = 1): void {
    this.ms += ticks * 1000;
  }
  now = (): number => this.ms;
}

const FAST = { flushMs: 5, pollMs: 25 };

async function renderAt(path: string, options: MockServerOptions = {}) {
  const mock = await startMockServer(0, "127.0.0.1", options);
  running.push(mock);
  window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
  window.history.replaceState(null, "", path);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<App baseUrl={mock.baseUrl} queryClient={queryClient} runSettings={FAST} />);
  return { user: userEvent.setup(), mock };
}

/** Starts a run of the http_smoke example straight through the API, before the page renders. */
async function startRunDirect(mock: RunningMockServer): Promise<string> {
  const response = await fetch(`${mock.baseUrl}/runs`, {
    method: "POST",
    headers: { Authorization: `Bearer ${MOCK_TOKEN}`, "Content-Type": "application/json" },
    body: JSON.stringify({ config_id: "cfg_http_smoke" }),
  });
  return ((await response.json()) as { id: string }).id;
}

async function expectNoAxeViolations() {
  const results = await axe.run(document.body, { rules: { "color-contrast": { enabled: false } } });
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
}

/** The text of a counter card, found by its label. */
function counter(label: string): string {
  const term = screen.getByText(label, { selector: "dt" });
  return term.nextElementSibling?.textContent ?? "";
}

/** The values that a chart line carries, in sample order. */
function lineValues(series: string): number[] {
  const line = document.querySelector(`polyline[data-series="${series}"]`);
  const values = line?.getAttribute("data-values") ?? "";
  return values === "" ? [] : values.split(",").map(Number);
}

function legend(series: string): string {
  return document.querySelector(`[data-legend="${series}"]`)?.textContent ?? "";
}

function feedStatus(): string {
  return document.querySelector(".feed-status")?.textContent ?? "";
}

/** The newest-first times of the table behind a chart, in seconds since midnight UTC. */
function tableSeconds(chart: string): number[] {
  const table = screen.getByRole("table", { name: `Newest values of ${chart}`, hidden: true });
  return within(table)
    .getAllByRole("rowheader", { hidden: true })
    .map((cell) => {
      const [h, m, s] = cell.textContent.split(":").map(Number);
      return (h ?? 0) * 3600 + (m ?? 0) * 60 + (s ?? 0);
    });
}

/** Replays a recorded stream into the view, an event at a time, split inside the events. */
function replay() {
  const pieces = FIXTURE.split("\n\n")
    .filter((piece) => piece !== "")
    .map((piece) => `${piece}\n\n`);
  const encoder = new TextEncoder();
  let controller: ReadableStreamDefaultController<Uint8Array> | undefined;
  const body = new ReadableStream<Uint8Array>({
    start(c) {
      controller = c;
    },
  });
  let next = 0;
  const original = globalThis.fetch.bind(globalThis);
  const requests: { url: string; headers: Headers }[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation((input, init) => {
    const url =
      typeof input === "string" ? input : input instanceof Request ? input.url : input.toString();
    if (!url.includes("/metrics:stream")) return original(input, init);
    requests.push({ url, headers: new Headers(init?.headers) });
    return Promise.resolve(
      new Response(body, { status: 200, headers: { "Content-Type": "text/event-stream" } }),
    );
  });
  return {
    pieces,
    requests,
    /** Sends the next `count` pieces, each in two chunks that cut it in the middle. */
    push(count: number): void {
      for (let i = 0; i < count; i += 1) {
        const piece = pieces[next];
        next += 1;
        if (piece === undefined) throw new Error("the recording has no more pieces");
        const middle = Math.floor(piece.length / 2);
        controller?.enqueue(encoder.encode(piece.slice(0, middle)));
        controller?.enqueue(encoder.encode(piece.slice(middle)));
      }
    },
    close(): void {
      controller?.close();
    },
  };
}

describe("recorded stream", () => {
  it("is a stream of the contract: retry, ten samples, one end event", () => {
    const pieces = FIXTURE.split("\n\n").filter((piece) => piece !== "");
    expect(pieces[0]).toBe("retry: 100");
    expect(pieces.slice(1, -1)).toHaveLength(10);
    for (const piece of pieces.slice(1, -1)) {
      expect(piece).toMatch(
        /^id: \d+\ndata: \{"ts":"2026-11-01T10:00:\d\d\.000Z","view":"application"/,
      );
    }
    expect(pieces[pieces.length - 1]).toBe('event: end\ndata: {"state":"completed"}');
  });
});

describe("live run view", () => {
  it("renders the counters and the chart points of a replayed stream, tick by tick", async () => {
    const clock = new Clock();
    const mock = await startMockServer(0, "127.0.0.1", { runs: { now: clock.now, runTicks: 10 } });
    running.push(mock);
    const id = await startRunDirect(mock);
    clock.advance(2);

    window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
    window.history.replaceState(null, "", `/runs/${id}`);
    const stream = replay();
    render(
      <App
        baseUrl={mock.baseUrl}
        queryClient={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
        runSettings={FAST}
      />,
    );

    expect(await screen.findByRole("heading", { level: 1, name: `Run ${id}` })).toBeDefined();
    await waitFor(() => {
      expect(stream.requests).toHaveLength(1);
    });
    // The stream carries the bearer token, since EventSource could not.
    expect(stream.requests[0]?.headers.get("Authorization")).toBe(`Bearer ${MOCK_TOKEN}`);
    expect(stream.requests[0]?.url).toContain("view=application");

    // The retry field and the first three ticks.
    stream.push(4);
    await waitFor(() => {
      expect(counter("Sessions attempted")).toBe("120");
    });
    expect(counter("Sessions succeeded")).toBe("88");
    expect(counter("Sessions failed")).toBe("2");
    expect(counter("Failed ratio")).toBe("1.67 %");
    expect(counter("Sessions per second")).toBe("60");
    expect(counter("Concurrent sessions")).toBe("30");
    expect(counter("Throughput")).toBe("1.97 Mbit/s");
    expect(lineValues("sessions.response_time.p50")).toEqual([13100, 13200, 13300]);
    expect(lineValues("sessions.response_time.p99")).toEqual([52400, 52800, 53200]);
    expect(lineValues("rate")).toEqual([20, 40, 60]);
    expect(legend("sessions.response_time.p99")).toBe("53.2 ms");
    expect(feedStatus()).toContain("Live");

    // The other seven ticks and the end event.
    stream.push(8);
    await waitFor(() => {
      expect(feedStatus()).toContain("The run ended");
    });
    expect(counter("Sessions attempted")).toBe("800");
    expect(counter("Sessions succeeded")).toBe("734");
    expect(counter("Sessions failed")).toBe("16");
    expect(counter("Failed ratio")).toBe("2 %");
    expect(counter("Sessions per second")).toBe("100");
    expect(counter("Concurrent sessions")).toBe("50");
    expect(counter("Throughput")).toBe("3.28 Mbit/s");
    expect(counter("Bytes sent")).toBe("1.64 MB");
    expect(counter("Bytes received")).toBe("6.55 MB");
    expect(lineValues("sessions.response_time.p50")).toEqual([
      13100, 13200, 13300, 13400, 13000, 13100, 13200, 13300, 13400, 13000,
    ]);
    expect(lineValues("sessions.response_time.p90")).toHaveLength(10);
    expect(lineValues("sessions.duration.p50")[9]).toBe(39000);
    expect(lineValues("rate")).toEqual([20, 40, 60, 80, 100, 100, 100, 100, 100, 100]);
    expect(lineValues("concurrent")).toEqual([10, 20, 30, 40, 50, 50, 50, 50, 50, 50]);
    expect(lineValues("bps")[9]).toBe(3276800);
    expect(legend("sessions.response_time.p50")).toBe("13.0 ms");
    expect(legend("sessions.response_time.p99")).toBe("52.0 ms");
    // The chart has a text alternative with the same points, newest first, one second apart.
    expect(tableSeconds("Response time")).toEqual(
      [10, 9, 8, 7, 6, 5, 4, 3, 2, 1].map((n) => 36000 + n),
    );

    // The end event makes the view read the run again, and the run is completed by then.
    clock.advance(10);
    await waitFor(() => {
      expect(
        within(screen.getByRole("region", { name: "State" })).getByText("completed"),
      ).toBeDefined();
    });
    stream.close();
    await expectNoAxeViolations();
  });

  it("starts a run from a stored configuration, shows its states, and stops it", async () => {
    const clock = new Clock();
    const { user, mock } = await renderAt("/runs", { runs: { now: clock.now, runTicks: 30 } });
    expect(
      await screen.findByText("No run exists yet. Start one with the form above."),
    ).toBeDefined();
    const start = screen.getByRole("button", { name: "Start run" });
    expect(start).toHaveProperty("disabled", true);
    await expectNoAxeViolations();

    const select = await screen.findByLabelText("Configuration");
    await waitFor(() => {
      expect(within(select).getAllByRole("option").length).toBeGreaterThan(1);
    });
    await user.selectOptions(select, "cfg_http_smoke");
    // The mock has one agent. Choosing none leaves the choice to the controller.
    expect(screen.getByLabelText("agent-local (online)")).toHaveProperty("checked", false);
    await user.click(screen.getByLabelText("agent-local (online)"));
    expect(start).toHaveProperty("disabled", false);
    await user.click(start);

    const heading = await screen.findByRole("heading", { level: 1, name: "Run run_mock0001" });
    expect(heading).toBeDefined();
    const stateSection = screen.getByRole("region", { name: "State" });
    expect(within(stateSection).getByText("pending", { selector: ".state" })).toBeDefined();
    expect(counter("Sessions attempted")).toBe("-");

    clock.advance();
    await waitFor(() => {
      expect(within(stateSection).getByText("preparing", { selector: ".state" })).toBeDefined();
    });
    clock.advance();
    await waitFor(() => {
      expect(within(stateSection).getByText("running", { selector: ".state" })).toBeDefined();
    });
    const transitions = within(stateSection).getByRole("list", { name: "State transitions" });
    expect(
      within(transitions)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["pending", "preparing", "running (current)"]);

    await user.click(screen.getByRole("button", { name: "Stop run" }));
    await waitFor(() => {
      expect(within(stateSection).getByText("stopping", { selector: ".state" })).toBeDefined();
    });
    expect(screen.queryByRole("button", { name: "Stop run" })).toBeNull();
    clock.advance();
    await waitFor(() => {
      expect(within(stateSection).getByText("aborted", { selector: ".state" })).toBeDefined();
    });
    expect(within(stateSection).getByText(/stopped by request/)).toBeDefined();
    expect(
      within(stateSection)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["pending", "preparing", "running", "stopping", "aborted (final)"]);
    expect(mock.baseUrl).toContain("/v1");

    // The run list shows the run and links to it.
    await user.click(screen.getByRole("link", { name: "All runs" }));
    const table = await screen.findByRole("table", { name: "Runs" });
    expect(within(table).getByRole("link", { name: "run_mock0001" })).toBeDefined();
    await waitFor(() => {
      expect(within(table).getByText("aborted")).toBeDefined();
    });
    expect(within(table).getByText("http_smoke")).toBeDefined();
  });

  it("preselects the configuration of the link and reports a start that the controller refuses", async () => {
    const { user } = await renderAt("/runs?config=cfg_dns_zone", {
      faults: [{ method: "POST", path: "/runs", status: 409, detail: "agent agent-local is busy" }],
    });
    const select = await screen.findByLabelText("Configuration");
    await waitFor(() => {
      expect(select).toHaveProperty("value", "cfg_dns_zone");
    });
    await user.click(screen.getByRole("button", { name: "Start run" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("agent agent-local is busy");
  });

  it("stops with force when the box is checked", async () => {
    const clock = new Clock();
    const { user, mock } = await renderAt("/runs", { runs: { now: clock.now, runTicks: 30 } });
    const id = await startRunDirect(mock);
    clock.advance(3);
    window.history.pushState(null, "", `/runs/${id}`);
    window.dispatchEvent(new PopStateEvent("popstate"));
    await screen.findByRole("heading", { level: 1, name: `Run ${id}` });
    await user.click(await screen.findByLabelText("Force: abort at once and skip ramp down"));
    await user.click(screen.getByRole("button", { name: "Stop run" }));
    await waitFor(() => {
      expect(
        within(screen.getByRole("region", { name: "State" })).getByText("aborted", {
          selector: ".state",
        }),
      ).toBeDefined();
    });
  });

  it("opens a run in the middle and fills the charts from the history", async () => {
    const clock = new Clock();
    const mock = await startMockServer(0, "127.0.0.1", {
      runs: { now: clock.now, runTicks: 30 },
      // The stream of this test never delivers, so only the history can fill the charts.
      faults: [{ path: /^\/runs\/[^/]+\/metrics:stream$/, status: 501 }],
    });
    running.push(mock);
    const id = await startRunDirect(mock);
    clock.advance(7);
    window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
    window.history.replaceState(null, "", `/runs/${id}`);
    render(
      <App
        baseUrl={mock.baseUrl}
        queryClient={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
        runSettings={FAST}
      />,
    );
    await waitFor(() => {
      expect(lineValues("rate")).toEqual([20, 40, 60, 80, 100, 100]);
    });
    expect(counter("Sessions attempted")).toBe("400");
  });

  it("reports a run that does not exist and an error of the run list", async () => {
    await renderAt("/runs/run_missing");
    expect((await screen.findByRole("alert")).textContent).toBe("No run has this id.");
    cleanup();
    await renderAt("/runs", { faults: [{ path: "/runs", status: 500, detail: "boom" }] });
    await waitFor(() => {
      expect(screen.getAllByRole("alert").some((alert) => alert.textContent.includes("boom"))).toBe(
        true,
      );
    });
  });
});

describe("feed resilience", () => {
  it("reconnects with Last-Event-ID after a dropped stream and fills the gap", async () => {
    const mock = await startMockServer(0, "127.0.0.1", {
      runs: { stepMs: 60, runTicks: 40 },
      streamDrops: 1,
    });
    running.push(mock);
    const id = await startRunDirect(mock);
    // Let the run reach tick 3, so the first connection sends events with ids before the cut.
    await new Promise((resolve) => setTimeout(resolve, 60 * 4));

    const original = globalThis.fetch.bind(globalThis);
    const streams: Headers[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((input, init) => {
      const url = typeof input === "string" ? input : input instanceof Request ? input.url : "";
      if (url.includes("/metrics:stream")) streams.push(new Headers(init?.headers));
      return original(input, init);
    });
    window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
    window.history.replaceState(null, "", `/runs/${id}`);
    render(
      <App
        baseUrl={mock.baseUrl}
        queryClient={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
        runSettings={{ flushMs: 5, pollMs: 60 }}
      />,
    );

    await waitFor(
      () => {
        expect(streams.length).toBeGreaterThanOrEqual(2);
      },
      { timeout: 4000 },
    );
    expect(streams[0]?.get("Last-Event-ID")).toBeNull();
    expect(streams[1]?.get("Last-Event-ID")).toMatch(/^\d+$/);
    await waitFor(
      () => {
        expect(feedStatus()).toContain("Live");
        expect(lineValues("rate").length).toBeGreaterThanOrEqual(8);
      },
      { timeout: 4000 },
    );
    // No tick is missing: the newest rows of the table follow one another by one second.
    const seconds = tableSeconds("Sessions per second");
    for (let i = 1; i < seconds.length; i += 1) {
      expect((seconds[i - 1] ?? 0) - (seconds[i] ?? 0)).toBe(1);
    }
  });

  it("polls the metrics endpoint when the controller answers the stream with 501", async () => {
    const mock = await startMockServer(0, "127.0.0.1", {
      runs: { stepMs: 40, runTicks: 10 },
      faults: [{ path: /^\/runs\/[^/]+\/metrics:stream$/, status: 501 }],
    });
    running.push(mock);
    const id = await startRunDirect(mock);
    window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
    window.history.replaceState(null, "", `/runs/${id}`);
    render(
      <App
        baseUrl={mock.baseUrl}
        queryClient={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
        runSettings={{ flushMs: 5, pollMs: 40 }}
      />,
    );
    await waitFor(() => {
      expect(feedStatus()).toContain("polls once per second");
    });
    await waitFor(
      () => {
        expect(feedStatus()).toContain("The run ended");
      },
      { timeout: 4000 },
    );
    expect(counter("Sessions attempted")).toBe("800");
    expect(lineValues("sessions.response_time.p50")).toHaveLength(10);
    expect(
      within(screen.getByRole("region", { name: "State" })).getByText("completed", {
        selector: ".state",
      }),
    ).toBeDefined();
    await expectNoAxeViolations();
  });

  it("signs out when the controller rejects the token of the stream", async () => {
    const mock = await startMockServer(0, "127.0.0.1", {
      runs: { stepMs: 40, runTicks: 10 },
      faults: [{ path: /^\/runs\/[^/]+\/metrics:stream$/, status: 401 }],
    });
    running.push(mock);
    const id = await startRunDirect(mock);
    window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
    window.history.replaceState(null, "", `/runs/${id}`);
    render(
      <App
        baseUrl={mock.baseUrl}
        queryClient={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
        runSettings={FAST}
      />,
    );
    // The mock sends 401 only for the stream, and the stream is read with the token of the session.
    expect(await screen.findByLabelText("API token")).toBeDefined();
  });
});

describe("mock data", () => {
  it("has the example configurations that the tests start", () => {
    const names = loadMockData().configs.map((config) => config.id);
    expect(names).toContain("cfg_http_smoke");
    expect(names).toContain("cfg_dns_zone");
  });
});
