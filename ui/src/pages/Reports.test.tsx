// SPDX-License-Identifier: GPL-3.0-only
//
// Component tests of the report view and the run comparison against the mock
// server. The runs follow the scripts of the mock: the first run of a server has no failed
// session and the later ones fail every 50th session, so a run of ten ticks of the
// http_smoke example attempts 800 sessions and fails 0 or 16 of them. The expected
// numbers are written out here, so a change of the mock or of the views shows as a failure.

import { QueryClient } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import axe from "axe-core";
import { afterEach, describe, expect, it } from "vitest";
import {
  MOCK_TOKEN,
  startMockServer,
  type MockServerOptions,
  type RunningMockServer,
} from "../../mock/server";
import { App } from "../App";

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

/** Run 1 has no failed session, every later run fails every 50th session. */
const SCRIPT: MockServerOptions["runs"] = { runTicks: 10, verdictStates: true };

function newServer(clock: Clock, options: MockServerOptions = {}) {
  return startMockServer(0, "127.0.0.1", {
    ...options,
    runs: {
      ...SCRIPT,
      now: clock.now,
      failedEvery: (ordinal) => (ordinal === 1 ? 0 : 50),
      ...options.runs,
    },
  });
}

async function startRunDirect(mock: RunningMockServer, configId = "cfg_http_smoke") {
  const response = await fetch(`${mock.baseUrl}/runs`, {
    method: "POST",
    headers: { Authorization: `Bearer ${MOCK_TOKEN}`, "Content-Type": "application/json" },
    body: JSON.stringify({ config_id: configId }),
  });
  return ((await response.json()) as { id: string }).id;
}

function show(mock: RunningMockServer, path: string) {
  if (!running.includes(mock)) running.push(mock);
  window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
  window.history.replaceState(null, "", path);
  render(
    <App
      baseUrl={mock.baseUrl}
      queryClient={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      runSettings={FAST}
    />,
  );
  return userEvent.setup();
}

async function expectNoAxeViolations() {
  const results = await axe.run(document.body, { rules: { "color-contrast": { enabled: false } } });
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
}

/** The cells of the row whose header has the given text, as plain text. */
function rowOf(table: HTMLElement, header: string): string[] {
  const row = within(table)
    .getAllByRole("row")
    .find((candidate) => within(candidate).queryByRole("rowheader", { name: header }) !== null);
  if (row === undefined) throw new Error(`no row ${header}`);
  return within(row)
    .getAllByRole("cell")
    .map((cell) => cell.textContent);
}

/** The text of a counter card, found by its label. */
function counter(label: string): string {
  return screen.getByText(label, { selector: "dt" }).nextElementSibling?.textContent ?? "";
}

describe("report of a passing run", () => {
  it("shows the verdict, the criteria, the realism results, and the totals", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    const id = await startRunDirect(mock);
    clock.advance(12);
    show(mock, `/runs/${id}/report`);

    expect(
      (await screen.findByRole("heading", { level: 1, name: `Report of run ${id}` })).textContent,
    ).toBe("Report of run run_mock0001");
    const verdict = await screen.findByRole("region", { name: "Verdict" });
    expect(within(verdict).getByText("Passed")).toBeDefined();
    expect(within(verdict).queryByText(/Reason:/)).toBeNull();

    const facts = screen.getByRole("region", { name: "Run" });
    expect(within(facts).getByText("completed")).toBeDefined();
    expect(within(facts).getByText("cfg_http_smoke")).toBeDefined();
    expect(within(facts).getByText("10 s")).toBeDefined();
    expect(within(facts).getByText("4711")).toBeDefined();

    const criteria = screen.getByRole("table", { name: "Criteria" });
    expect(rowOf(criteria, "sessions.failed_ratio == 0")).toEqual([
      "0",
      "Passed",
      "observed 0 == 0 holds",
    ]);

    const realism = screen.getByRole("table", { name: "Realism results" });
    expect(rowOf(realism, "inter_arrival.ks_statistic")).toEqual([
      "application=all, property=inter_arrival",
      "0.03",
      "<= 0.05",
      "Passed",
    ]);
    expect(rowOf(realism, "response_bytes.ks_statistic")).toEqual([
      "application=all, property=response_bytes",
      "0.04",
      "<= 0.05",
      "Passed",
    ]);

    expect(counter("Sessions attempted")).toBe("800");
    expect(counter("Sessions succeeded")).toBe("750");
    expect(counter("Sessions failed")).toBe("0");
    expect(counter("Failed ratio")).toBe("0 %");
    expect(counter("Bytes sent")).toBe("1.64 MB");
    await expectNoAxeViolations();
  });
});

describe("report of a failing run", () => {
  it("shows the failed verdict with its reason, the failed criterion, and the failed realism result", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    await startRunDirect(mock);
    const id = await startRunDirect(mock);
    clock.advance(12);
    show(mock, `/runs/${id}/report`);

    const verdict = await screen.findByRole("region", { name: "Verdict" });
    expect(within(verdict).getByText("Failed")).toBeDefined();
    expect(verdict.textContent).toContain(
      "Reason: criteria failed: sessions.failed_ratio: observed 0.02 == 0 does not hold",
    );
    expect(within(screen.getByRole("region", { name: "Run" })).getByText("failed")).toBeDefined();

    const criteria = screen.getByRole("table", { name: "Criteria" });
    expect(rowOf(criteria, "sessions.failed_ratio == 0")).toEqual([
      "0.02",
      "Failed",
      "observed 0.02 == 0 does not hold",
    ]);
    const realism = screen.getByRole("table", { name: "Realism results" });
    expect(rowOf(realism, "response_bytes.ks_statistic")).toEqual([
      "application=all, property=response_bytes",
      "0.08",
      "<= 0.05",
      "Failed",
    ]);
    expect(rowOf(realism, "request_bytes.ks_statistic")).toEqual([
      "application=all, property=request_bytes",
      "0.02",
      "<= 0.05",
      "Passed",
    ]);

    expect(counter("Sessions failed")).toBe("16");
    expect(counter("Failed ratio")).toBe("2 %");
    expect(counter("Sessions succeeded")).toBe("734");
    await expectNoAxeViolations();
  });

  it("shows a criterion without a value as no value and not as a failure", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    // The mock knows no value for a mix error, so the second criterion of the example has none.
    const id = await startRunDirect(mock, "cfg_enterprise_mix_10g");
    clock.advance(12);
    show(mock, `/runs/${id}/report`);
    const verdict = await screen.findByRole("region", { name: "Verdict" });
    expect(within(verdict).getByText("Passed")).toBeDefined();
    const criteria = screen.getByRole("table", { name: "Criteria" });
    expect(rowOf(criteria, "sessions.failed_ratio < 0.001")).toEqual([
      "0",
      "Passed",
      "observed 0 < 0.001 holds",
    ]);
    expect(rowOf(criteria, "mix.max_abs_error_pp <= 2")).toEqual([
      "-",
      "No value",
      "metric mix.max_abs_error_pp has no value",
    ]);
  });
});

describe("report of a run without a verdict", () => {
  it("says that a run in progress has no result yet", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    const id = await startRunDirect(mock);
    clock.advance(4);
    show(mock, `/runs/${id}/report`);
    const verdict = await screen.findByRole("region", { name: "Verdict" });
    expect(within(verdict).getByText("No verdict")).toBeDefined();
    expect(within(verdict).getByText(/has not ended/)).toBeDefined();
    expect(screen.getByText("No criterion has a result yet.")).toBeDefined();
    expect(screen.getByText("No realism result exists yet.")).toBeDefined();
  });

  it("says that an aborted run has no verdict and no realism results", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    const id = await startRunDirect(mock);
    clock.advance(4);
    await fetch(`${mock.baseUrl}/runs/${id}:stop`, {
      method: "POST",
      headers: { Authorization: `Bearer ${MOCK_TOKEN}`, "Content-Type": "application/json" },
      body: JSON.stringify({ force: true }),
    });
    show(mock, `/runs/${id}/report`);
    const verdict = await screen.findByRole("region", { name: "Verdict" });
    expect(within(verdict).getByText("The controller gave this run no verdict.")).toBeDefined();
    expect(verdict.textContent).toContain("Reason: stopped by request");
    expect(screen.getByText("The report holds no realism results for this run.")).toBeDefined();
  });
});

describe("report errors and links", () => {
  it("reports a run that does not exist", async () => {
    const mock = await newServer(new Clock());
    show(mock, "/runs/run_missing/report");
    expect((await screen.findByRole("alert")).textContent).toBe("No run has this id.");
  });

  it("reports an error of the controller", async () => {
    const mock = await newServer(new Clock(), {
      faults: [{ path: /^\/runs\/[^/]+\/report$/, status: 500, detail: "boom" }],
    });
    show(mock, "/runs/run_mock0001/report");
    expect((await screen.findByRole("alert")).textContent).toBe(
      "The report could not be loaded. boom",
    );
  });

  it("signs out when the controller rejects the token", async () => {
    const mock = await newServer(new Clock(), {
      faults: [{ path: /^\/runs\/[^/]+\/report$/, status: 401 }],
    });
    show(mock, "/runs/run_mock0001/report");
    expect(await screen.findByText(/no longer accepts the token/)).toBeDefined();
  });

  it("opens the report from the run list and the run view", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    const id = await startRunDirect(mock);
    clock.advance(12);
    const user = show(mock, "/runs");
    await user.click(await screen.findByRole("link", { name: `Report of ${id}` }));
    expect(await screen.findByRole("heading", { name: `Report of run ${id}` })).toBeDefined();
    expect(window.location.pathname).toBe(`/runs/${id}/report`);
    await user.click(screen.getByRole("link", { name: "Live view" }));
    await screen.findByRole("heading", { name: `Run ${id}` });
    await user.click(screen.getByRole("link", { name: "Report" }));
    expect(await screen.findByRole("heading", { name: `Report of run ${id}` })).toBeDefined();
  });
});

describe("comparison of two runs", () => {
  async function twoRuns() {
    const clock = new Clock();
    const mock = await newServer(clock);
    const a = await startRunDirect(mock);
    const b = await startRunDirect(mock);
    clock.advance(12);
    return { mock, a, b };
  }

  it("shows the totals, criteria, and realism results of both runs and what got worse", async () => {
    const { mock, a, b } = await twoRuns();
    show(mock, `/compare?a=${a}&b=${b}`);

    const summary = await screen.findByRole("region", { name: "Summary" });
    expect(within(summary).getByRole("status").textContent).toBe(
      "2 criteria or realism results got worse from run A to run B.",
    );

    const facts = screen.getByRole("table", { name: "Runs compared" });
    expect(rowOf(facts, "State")).toEqual(["completed", "failed"]);
    expect(rowOf(facts, "Verdict")).toEqual(["Passed", "Failed"]);
    expect(rowOf(facts, "Duration")).toEqual(["10 s", "10 s"]);
    expect(
      within(facts)
        .getAllByRole("columnheader")
        .map((cell) => cell.textContent),
    ).toEqual(["Property", `Run A: ${a}`, `Run B: ${b}`]);

    const totals = screen.getByRole("table", { name: "Totals compared" });
    expect(rowOf(totals, "Sessions attempted")).toEqual(["800", "800", "0 (0 %)"]);
    expect(rowOf(totals, "Sessions succeeded")).toEqual(["750", "734", "−16 (−2.1 %)"]);
    expect(rowOf(totals, "Sessions failed")).toEqual(["0", "16", "+16"]);
    expect(rowOf(totals, "Failed ratio")).toEqual(["0 %", "2 %", "+2 pp"]);
    expect(rowOf(totals, "Bytes sent")).toEqual(["1.64 MB", "1.64 MB", "0 B (0 %)"]);

    const criteria = screen.getByRole("table", { name: "Criteria compared" });
    expect(rowOf(criteria, "sessions.failed_ratio == 0")).toEqual([
      "Passed (observed 0)",
      "Failed (observed 0.02)",
      "Now fails",
    ]);
    const realism = screen.getByRole("table", { name: "Realism compared" });
    expect(rowOf(realism, "response_bytes.ks_statistic")).toEqual([
      "application=all, property=response_bytes",
      "<= 0.05",
      "Passed (observed 0.04)",
      "Failed (observed 0.08)",
      "Now fails",
    ]);
    expect(rowOf(realism, "inter_arrival.ks_statistic")).toEqual([
      "application=all, property=inter_arrival",
      "<= 0.05",
      "Passed (observed 0.03)",
      "Passed (observed 0.03)",
      "Same",
    ]);
    await expectNoAxeViolations();
  });

  it("reads the change the other way when the runs are swapped", async () => {
    const { mock, a, b } = await twoRuns();
    show(mock, `/compare?a=${b}&b=${a}`);
    const summary = await screen.findByRole("region", { name: "Summary" });
    expect(within(summary).getByRole("status").textContent).toBe(
      "No criterion or realism result got worse from run A to run B.",
    );
    const criteria = screen.getByRole("table", { name: "Criteria compared" });
    expect(rowOf(criteria, "sessions.failed_ratio == 0")[2]).toBe("Now passes");
    const totals = screen.getByRole("table", { name: "Totals compared" });
    expect(rowOf(totals, "Sessions failed")).toEqual(["16", "0", "−16 (−100 %)"]);
  });

  it("marks criteria that only one run has, when the configurations differ", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    const a = await startRunDirect(mock);
    const b = await startRunDirect(mock, "cfg_filtered_criteria");
    clock.advance(12);
    show(mock, `/compare?a=${a}&b=${b}`);
    await screen.findByRole("region", { name: "Summary" });
    const criteria = screen.getByRole("table", { name: "Criteria compared" });
    expect(rowOf(criteria, "sessions.failed_ratio == 0")).toEqual([
      "Passed (observed 0)",
      "Not in this run",
      "Only in run A",
    ]);
    expect(rowOf(criteria, "sessions.failed_ratio < 0.01")).toEqual([
      "Not in this run",
      "Failed (observed 0.01875)",
      "Only in run B",
    ]);
    // The two mix criteria of the filtered example differ in their limit and their filter.
    expect(rowOf(criteria, "mix.abs_error_pp <= 1 where application=http1")).toEqual([
      "Not in this run",
      "No value",
      "Only in run B",
    ]);
    expect(rowOf(criteria, "mix.abs_error_pp <= 3 where application=dns")[2]).toBe("Only in run B");
  });

  it("chooses the runs with the two selects and keeps them in the link", async () => {
    const { mock, a, b } = await twoRuns();
    const user = show(mock, "/compare");
    expect(await screen.findByText("Choose two runs to compare their reports.")).toBeDefined();
    const selectA = await screen.findByLabelText("Run A");
    await waitFor(() => {
      expect(within(selectA).getAllByRole("option")).toHaveLength(3);
    });
    await user.selectOptions(selectA, a);
    await user.selectOptions(screen.getByLabelText("Run B"), b);
    expect(window.location.search).toBe(`?a=${a}&b=${b}`);
    expect(await screen.findByRole("table", { name: "Totals compared" })).toBeDefined();
    await user.selectOptions(screen.getByLabelText("Run B"), a);
    expect((await screen.findByRole("alert")).textContent).toBe("Choose two different runs.");
  });

  it("preselects the run of a link from the report and reports a run that does not exist", async () => {
    const { mock, a } = await twoRuns();
    const user = show(mock, `/runs/${a}/report`);
    await user.click(await screen.findByRole("link", { name: "Compare with another run" }));
    const selectA = await screen.findByLabelText("Run A");
    await waitFor(() => {
      expect(selectA).toHaveProperty("value", a);
    });
    cleanup();
    show(mock, `/compare?a=${a}&b=run_missing`);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("Run B (run_missing): no run has this id.");
  });

  it("states that a run in progress has no verdict", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    const a = await startRunDirect(mock);
    clock.advance(12);
    const b = await startRunDirect(mock);
    clock.advance(3);
    show(mock, `/compare?a=${a}&b=${b}`);
    await screen.findByRole("region", { name: "Summary" });
    expect(screen.getByText(/has not ended/)).toBeDefined();
    const facts = screen.getByRole("table", { name: "Runs compared" });
    expect(rowOf(facts, "Verdict")).toEqual(["Passed", "No verdict"]);
  });
});

/**
 * The filtered example with two criteria that agree in metric, operator, and value and differ
 * only in their filter. Run 1 of a server has no failed session and run 2 fails every 50th
 * session of each application: http1/web_get attempts 720 sessions and fails 14, which is
 * 0.0194444 of them, and dns/lookup attempts 80 and fails 1, which is 0.0125.
 */
async function postFilteredConfig(mock: RunningMockServer): Promise<string> {
  const headers = { Authorization: `Bearer ${MOCK_TOKEN}`, "Content-Type": "application/json" };
  const base = (await (
    await fetch(`${mock.baseUrl}/configs/cfg_filtered_criteria`, { headers })
  ).json()) as { metadata: { name: string }; criteria: unknown };
  const body = {
    ...base,
    metadata: { ...base.metadata, name: "two_filters" },
    criteria: [
      { metric: "sessions.failed_ratio", op: "<=", value: 0.015, where: { application: "http1" } },
      { metric: "sessions.failed_ratio", op: "<=", value: 0.015, where: { application: "dns" } },
    ],
  };
  const response = await fetch(`${mock.baseUrl}/configs`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  expect(response.status).toBe(201);
  return ((await response.json()) as { id: string }).id;
}

describe("report with scenarios and criteria that differ in their filter", () => {
  it("shows the scenario table with the counts of each scenario", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    await startRunDirect(mock);
    const id = await startRunDirect(mock, "cfg_filtered_criteria");
    clock.advance(12);
    show(mock, `/runs/${id}/report`);

    const section = await screen.findByRole("region", { name: "Scenarios" });
    const table = within(section).getByRole("table", { name: "Scenarios" });
    expect(
      within(table)
        .getAllByRole("columnheader")
        .map((cell) => cell.textContent),
    ).toEqual([
      "Scenario",
      "Sessions attempted",
      "Sessions succeeded",
      "Sessions failed",
      "Failed ratio",
      "Bytes sent",
      "Bytes received",
    ]);
    expect(rowOf(table, "dns/lookup")).toEqual([
      "80",
      "74",
      "1",
      "1.25 %",
      "163.84 kB",
      "655.36 kB",
    ]);
    expect(rowOf(table, "http1/web_get")).toEqual([
      "720",
      "661",
      "14",
      "1.94 %",
      "1.47 MB",
      "5.9 MB",
    ]);
    // The rows add up to the totals of the run.
    expect(counter("Sessions attempted")).toBe("800");
    expect(counter("Sessions failed")).toBe("15");
    await expectNoAxeViolations();
  });

  it("lists two criteria that differ only in their filter as two rows", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    await startRunDirect(mock);
    const config = await postFilteredConfig(mock);
    const id = await startRunDirect(mock, config);
    clock.advance(12);
    show(mock, `/runs/${id}/report`);

    const criteria = await screen.findByRole("table", { name: "Criteria" });
    expect(rowOf(criteria, "sessions.failed_ratio <= 0.015 where application=http1")).toEqual([
      "0.0194444",
      "Failed",
      "observed 0.019444444444444445 <= 0.015 does not hold",
    ]);
    expect(rowOf(criteria, "sessions.failed_ratio <= 0.015 where application=dns")).toEqual([
      "0.0125",
      "Passed",
      "observed 0.0125 <= 0.015 holds",
    ]);
    await expectNoAxeViolations();
  });

  it("says that a run that has not started has no scenario counts", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    const id = await startRunDirect(mock);
    show(mock, `/runs/${id}/report`);
    expect(await screen.findByText("No scenario has counts yet.")).toBeDefined();
  });
});

describe("comparison with scenarios and criteria that differ in their filter", () => {
  async function filteredRuns() {
    const clock = new Clock();
    const mock = await newServer(clock);
    const config = await postFilteredConfig(mock);
    const a = await startRunDirect(mock, config);
    const b = await startRunDirect(mock, config);
    clock.advance(12);
    return { mock, a, b };
  }

  it("pairs the criteria by their filter and compares the scenarios side by side", async () => {
    const { mock, a, b } = await filteredRuns();
    show(mock, `/compare?a=${a}&b=${b}`);

    const summary = await screen.findByRole("region", { name: "Summary" });
    // One criterion and the realism result of the response bytes got worse.
    expect(within(summary).getByRole("status").textContent).toBe(
      "2 criteria or realism results got worse from run A to run B.",
    );

    const criteria = screen.getByRole("table", { name: "Criteria compared" });
    expect(rowOf(criteria, "sessions.failed_ratio <= 0.015 where application=http1")).toEqual([
      "Passed (observed 0)",
      "Failed (observed 0.0194444)",
      "Now fails",
    ]);
    expect(rowOf(criteria, "sessions.failed_ratio <= 0.015 where application=dns")).toEqual([
      "Passed (observed 0)",
      "Passed (observed 0.0125)",
      "Same",
    ]);

    const scenarios = screen.getByRole("table", { name: "Scenarios compared" });
    expect(rowOf(scenarios, "dns/lookup")).toEqual([
      "80 / 80",
      "0 / 1",
      "0 % / 1.25 %",
      "163.84 kB / 163.84 kB",
      "655.36 kB / 655.36 kB",
      "+1.25 pp",
    ]);
    expect(rowOf(scenarios, "http1/web_get")).toEqual([
      "720 / 720",
      "0 / 14",
      "0 % / 1.94 %",
      "1.47 MB / 1.47 MB",
      "5.9 MB / 5.9 MB",
      "+1.94 pp",
    ]);
    await expectNoAxeViolations();
  });

  it("marks a scenario that only one run has", async () => {
    const clock = new Clock();
    const mock = await newServer(clock);
    const a = await startRunDirect(mock);
    const b = await startRunDirect(mock, "cfg_filtered_criteria");
    clock.advance(12);
    show(mock, `/compare?a=${a}&b=${b}`);
    await screen.findByRole("region", { name: "Summary" });
    const scenarios = screen.getByRole("table", { name: "Scenarios compared" });
    expect(rowOf(scenarios, "dns/lookup")).toEqual([
      "- / 80",
      "- / 1",
      "- / 1.25 %",
      "- / 163.84 kB",
      "- / 655.36 kB",
      "Only in run B",
    ]);
    // The same scenario in both runs pairs up although the mix differs.
    expect(rowOf(scenarios, "http1/web_get")[0]).toBe("800 / 720");
  });
});
