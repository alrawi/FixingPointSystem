// SPDX-License-Identifier: GPL-3.0-only

import { describe, expect, it } from "vitest";
import type { CriterionResult, RunReport } from "../api/client";
import {
  compareReports,
  criterionLabel,
  criterionOutcome,
  durationMs,
  pairCriteria,
  pairRealism,
  totalChange,
  totalsOf,
  verdictOutcome,
} from "./compare";

const failedRatio = (value: number, extra: Partial<CriterionResult> = {}): CriterionResult => ({
  metric: "sessions.failed_ratio",
  op: "<",
  value,
  ...extra,
});

function report(criteria: CriterionResult[] | undefined, passed?: boolean): RunReport {
  return {
    run: {
      id: "r",
      config_id: "c",
      state: "completed",
      ...(criteria === undefined
        ? {}
        : { verdict: { criteria, ...(passed === undefined ? {} : { passed }) } }),
    },
    realism: [],
  };
}

describe("outcomes", () => {
  it("reads passed, failed, and no value from the criterion", () => {
    expect(criterionOutcome(failedRatio(0.01, { observed: 0, passed: true }))).toBe("pass");
    expect(criterionOutcome(failedRatio(0.01, { observed: 1, passed: false }))).toBe("fail");
    expect(criterionOutcome(failedRatio(0.01, { reason: "metric has no value" }))).toBe("none");
    expect(criterionOutcome(undefined)).toBe("none");
  });

  it("treats a criterion that cannot be evaluated as failed and one without a verdict as none", () => {
    expect(criterionOutcome({ metric: "x.y", passed: false })).toBe("fail");
    expect(verdictOutcome({ id: "r", config_id: "c", state: "running" })).toBe("none");
    expect(
      verdictOutcome({ id: "r", config_id: "c", state: "failed", verdict: { passed: false } }),
    ).toBe("fail");
  });

  it("writes a criterion as the configuration states it", () => {
    expect(criterionLabel(failedRatio(0.01))).toBe("sessions.failed_ratio < 0.01");
    expect(criterionLabel({ op: "==" })).toBe("(unnamed metric) ==");
  });
});

describe("totals", () => {
  it("lists nine totals and derives the failed ratio", () => {
    const totals = totalsOf({
      id: "r",
      config_id: "c",
      state: "completed",
      summary: { sessions_attempted: 800, sessions_failed: 16, sessions_succeeded: 734 },
    });
    expect(totals.map((total) => total.id)).toEqual([
      "attempted",
      "succeeded",
      "failed",
      "ratio",
      "rate",
      "concurrent",
      "throughput",
      "bytes_tx",
      "bytes_rx",
    ]);
    expect(totals.find((total) => total.id === "ratio")?.value).toBe(0.02);
    expect(totals.find((total) => total.id === "rate")?.value).toBeUndefined();
  });

  it("has no failed ratio for a run without attempts", () => {
    const totals = totalsOf({
      id: "r",
      config_id: "c",
      state: "pending",
      summary: { sessions_attempted: 0, sessions_failed: 0 },
    });
    expect(totals.find((total) => total.id === "ratio")?.value).toBeUndefined();
  });

  it("computes the change with a sign and a relative part", () => {
    const [attempted, , , ratio] = totalsOf({ id: "r", config_id: "c", state: "completed" });
    if (attempted === undefined || ratio === undefined) throw new Error("missing totals");
    expect(totalChange(attempted, 800, 1000)).toEqual({ delta: "+200", percent: "+25 %" });
    expect(totalChange(attempted, 1000, 750)).toEqual({ delta: "−250", percent: "−25 %" });
    expect(totalChange(attempted, 0, 5)).toEqual({ delta: "+5", percent: "" });
    expect(totalChange(attempted, 5, 5)).toEqual({ delta: "0", percent: "0 %" });
    expect(totalChange(attempted, undefined, 5)).toEqual({ delta: "-", percent: "" });
    expect(totalChange(ratio, 0.02, 0.005)).toEqual({ delta: "−1.5 pp", percent: "" });
  });
});

describe("pairing", () => {
  it("pairs criteria by metric, operator, and value and keeps the order of run A first", () => {
    const rows = pairCriteria(
      [failedRatio(0.01, { passed: true, observed: 0 }), failedRatio(0.5, { passed: true })],
      [failedRatio(0.5, { passed: false }), failedRatio(0.01, { passed: false, observed: 1 })],
    );
    expect(rows.map((row) => [row.label, row.change])).toEqual([
      ["sessions.failed_ratio < 0.01", "regressed"],
      ["sessions.failed_ratio < 0.5", "regressed"],
    ]);
  });

  it("pairs criteria that agree in every field in order, because the API omits the where filter", () => {
    const rows = pairCriteria(
      [failedRatio(1, { passed: true }), failedRatio(1, { passed: false })],
      [failedRatio(1, { passed: true }), failedRatio(1, { passed: true })],
    );
    expect(rows.map((row) => row.change)).toEqual(["same", "improved"]);
  });

  it("marks criteria that one run lacks and combinations of no value as different", () => {
    const rows = pairCriteria(
      [failedRatio(1, { passed: true }), failedRatio(2)],
      [failedRatio(2, { passed: true }), failedRatio(3, { passed: true })],
    );
    expect(rows.map((row) => [row.label, row.change])).toEqual([
      ["sessions.failed_ratio < 1", "only-a"],
      ["sessions.failed_ratio < 2", "differs"],
      ["sessions.failed_ratio < 3", "only-b"],
    ]);
  });

  it("pairs realism results by property and joins differing targets", () => {
    const rows = pairRealism(
      [
        { property: "a", observed: 1, target: "<= 2", passed: true },
        { property: "b", observed: 1, target: "<= 2", passed: true },
      ],
      [
        { property: "b", observed: 3, target: "<= 2.5", passed: false },
        { property: "c", observed: 1, target: ">= 0", passed: true },
      ],
    );
    expect(rows.map((row) => [row.property, row.target, row.change])).toEqual([
      ["a", "<= 2", "only-a"],
      ["b", "<= 2 / <= 2.5", "regressed"],
      ["c", ">= 0", "only-b"],
    ]);
  });

  it("counts the results that got worse", () => {
    const comparison = compareReports(
      report([failedRatio(1, { passed: true })], true),
      report([failedRatio(1, { passed: false })], false),
    );
    expect(comparison.regressions).toBe(1);
    expect(compareReports(report(undefined), report(undefined))).toEqual({
      criteria: [],
      realism: [],
      regressions: 0,
    });
  });
});

describe("duration", () => {
  it("needs a start and an end", () => {
    const run = { id: "r", config_id: "c", state: "completed" as const };
    expect(durationMs(run)).toBeUndefined();
    expect(
      durationMs({ ...run, started_at: "2026-11-01T10:00:00Z", ended_at: "2026-11-01T10:01:05Z" }),
    ).toBe(65_000);
  });
});
