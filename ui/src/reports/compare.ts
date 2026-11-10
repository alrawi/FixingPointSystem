// SPDX-License-Identifier: GPL-3.0-only
//
// The model of a report and of the comparison of two reports. These functions
// are pure, so the views only draw what they return and the tests state
// literal values.

import type { CriterionResult, RealismResult, Run, RunReport, ScenarioTotals } from "../api/client";
import {
  formatBitsPerSecond,
  formatBytes,
  formatCount,
  formatDecimal,
  formatPercent,
} from "../runs/format";

/** How a criterion or a verdict came out. `none` means that nothing was compared. */
export type Outcome = "pass" | "fail" | "none";

/** The outcome of one criterion. A criterion without `passed` compared no value. */
export function criterionOutcome(result: CriterionResult | undefined): Outcome {
  if (result?.passed === undefined) return "none";
  return result.passed ? "pass" : "fail";
}

/** The outcome of the whole run. A run without a verdict has not ended, or ended without one. */
export function verdictOutcome(run: Run): Outcome {
  const passed = run.verdict?.passed;
  if (passed === undefined) return "none";
  return passed ? "pass" : "fail";
}

/** The words for an outcome of a criterion. */
export function criterionWord(outcome: Outcome): string {
  return outcome === "pass" ? "Passed" : outcome === "fail" ? "Failed" : "No value";
}

/** The words for the verdict of a run. */
export function verdictWord(outcome: Outcome): string {
  return outcome === "pass" ? "Passed" : outcome === "fail" ? "Failed" : "No verdict";
}

/** A label set as text with the names in alphabetical order, such as `application=dns, side=server`. */
export function labelsText(labels: Readonly<Record<string, string>> | undefined): string {
  return Object.entries(labels ?? {})
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([name, value]) => `${name}=${value}`)
    .join(", ");
}

/**
 * A criterion as the configuration states it, such as `sessions.failed_ratio < 0.01`, followed
 * by its label filter when it has one, such as `mix.abs_error_pp <= 1 where application=http1`.
 */
export function criterionLabel(result: CriterionResult): string {
  const parts = [result.metric ?? "(unnamed metric)"];
  if (result.op !== undefined) parts.push(result.op);
  if (result.value !== undefined) parts.push(String(result.value));
  const where = labelsText(result.where);
  if (where !== "") parts.push("where", where);
  return parts.join(" ");
}

/** The run summary as a list of totals, with the failed ratio that the summary lacks. */
export interface Total {
  id: string;
  label: string;
  value: number | undefined;
  /** Formats a value of this total. */
  format: (value: number | undefined) => string;
  /** Formats a difference of two values of this total, without a sign. */
  formatDelta: (value: number) => string;
}

function formatPoints(value: number): string {
  return `${new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(value * 100)} pp`;
}

/** The totals of a run in display order. */
export function totalsOf(run: Run): Total[] {
  const summary = run.summary;
  const attempted = summary?.sessions_attempted;
  const failed = summary?.sessions_failed;
  const ratio =
    attempted !== undefined && failed !== undefined && attempted > 0
      ? failed / attempted
      : undefined;
  return [
    {
      id: "attempted",
      label: "Sessions attempted",
      value: attempted,
      format: formatCount,
      formatDelta: formatCount,
    },
    {
      id: "succeeded",
      label: "Sessions succeeded",
      value: summary?.sessions_succeeded,
      format: formatCount,
      formatDelta: formatCount,
    },
    {
      id: "failed",
      label: "Sessions failed",
      value: failed,
      format: formatCount,
      formatDelta: formatCount,
    },
    {
      id: "ratio",
      label: "Failed ratio",
      value: ratio,
      format: formatPercent,
      formatDelta: formatPoints,
    },
    {
      id: "rate",
      label: "Sessions per second",
      value: summary?.sessions_per_second,
      format: formatDecimal,
      formatDelta: formatDecimal,
    },
    {
      id: "concurrent",
      label: "Concurrent sessions",
      value: summary?.concurrent_sessions,
      format: formatCount,
      formatDelta: formatCount,
    },
    {
      id: "throughput",
      label: "Throughput",
      value: summary?.throughput_bps,
      format: formatBitsPerSecond,
      formatDelta: formatBitsPerSecond,
    },
    {
      id: "bytes_tx",
      label: "Bytes sent",
      value: summary?.bytes_tx,
      format: formatBytes,
      formatDelta: formatBytes,
    },
    {
      id: "bytes_rx",
      label: "Bytes received",
      value: summary?.bytes_rx,
      format: formatBytes,
      formatDelta: formatBytes,
    },
  ];
}

/** The change from run A to run B of one total, as text. */
export interface TotalChange {
  /** For example `+1,200`, or `-` when either run lacks the value. */
  delta: string;
  /** The relative change, for example `+12.5 %`, or empty when A is zero or a value is missing. */
  percent: string;
}

function sign(value: number): string {
  return value > 0 ? "+" : value < 0 ? "−" : "";
}

/** Computes the change from `a` to `b`. */
export function totalChange(
  total: Total,
  a: number | undefined,
  b: number | undefined,
): TotalChange {
  if (a === undefined || b === undefined) return { delta: "-", percent: "" };
  const diff = b - a;
  const delta = `${sign(diff)}${total.formatDelta(Math.abs(diff))}`;
  if (a === 0 || total.id === "ratio") return { delta, percent: "" };
  const relative = (diff / Math.abs(a)) * 100;
  const text = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 }).format(
    Math.abs(relative),
  );
  return { delta, percent: `${sign(relative)}${text} %` };
}

/** How a row differs between the two runs. */
export type Change = "same" | "regressed" | "improved" | "differs" | "only-a" | "only-b";

/** The words for a change. */
export const CHANGE_WORD: Record<Change, string> = {
  same: "Same",
  regressed: "Now fails",
  improved: "Now passes",
  differs: "Differs",
  "only-a": "Only in run A",
  "only-b": "Only in run B",
};

function changeOf(a: Outcome | undefined, b: Outcome | undefined): Change {
  if (a === undefined) return "only-b";
  if (b === undefined) return "only-a";
  if (a === b) return "same";
  if (a === "pass" && b === "fail") return "regressed";
  if (a === "fail" && b === "pass") return "improved";
  return "differs";
}

/** One criterion in the comparison. A side is undefined when that run lacks the criterion. */
export interface CriterionRow {
  key: string;
  label: string;
  a: CriterionResult | undefined;
  b: CriterionResult | undefined;
  change: Change;
}

/**
 * Pairs the criteria of two runs. A criterion is the same in both runs when metric, operator,
 * value, and `where` filter agree. Criteria that agree in all four, which a configuration can
 * state twice, are paired in order of appearance.
 */
export function pairCriteria(
  a: readonly CriterionResult[],
  b: readonly CriterionResult[],
): CriterionRow[] {
  const keyed = (list: readonly CriterionResult[]) => {
    const seen = new Map<string, number>();
    return list.map((result) => {
      const base = criterionLabel(result);
      const count = seen.get(base) ?? 0;
      seen.set(base, count + 1);
      return { key: `${base}#${String(count)}`, result };
    });
  };
  const left = keyed(a);
  const right = keyed(b);
  const rows: CriterionRow[] = left.map(({ key, result }) => {
    const other = right.find((candidate) => candidate.key === key)?.result;
    return {
      key,
      label: criterionLabel(result),
      a: result,
      b: other,
      change: changeOf(
        criterionOutcome(result),
        other === undefined ? undefined : criterionOutcome(other),
      ),
    };
  });
  for (const { key, result } of right) {
    if (left.some((candidate) => candidate.key === key)) continue;
    rows.push({ key, label: criterionLabel(result), a: undefined, b: result, change: "only-b" });
  }
  return rows;
}

/** One realism result in the comparison. */
export interface RealismRow {
  /** What pairs the results: the metric with its labels, or the property for a result without a metric. */
  key: string;
  property: string;
  /** The labels of the series as text, empty when the result has none. */
  labels: string;
  a: RealismResult | undefined;
  b: RealismResult | undefined;
  /** The target of the property, with both targets when the runs differ. */
  target: string;
  change: Change;
}

function realismOutcome(result: RealismResult | undefined): Outcome | undefined {
  if (result === undefined) return undefined;
  return result.passed ? "pass" : "fail";
}

/**
 * The key that pairs a realism result across runs: the metric with its labels, and the property
 * when the result has no metric. The property text may change between versions of the runner,
 * so it is not part of a key that the metric supplies.
 */
export function realismKey(result: RealismResult): string {
  if (result.metric === undefined) return `property ${result.property}`;
  return `metric ${result.metric} ${labelsText(result.labels)}`;
}

/** Pairs the realism results of two runs by metric and labels, and by property without a metric. */
export function pairRealism(
  a: readonly RealismResult[],
  b: readonly RealismResult[],
): RealismRow[] {
  const keys = [...new Set([...a.map(realismKey), ...b.map(realismKey)])];
  return keys.map((key) => {
    const left = a.find((result) => realismKey(result) === key);
    const right = b.find((result) => realismKey(result) === key);
    const shown = left ?? right;
    const target =
      left !== undefined && right !== undefined && left.target !== right.target
        ? `${left.target} / ${right.target}`
        : (shown?.target ?? "");
    return {
      key,
      property: shown?.property ?? "",
      labels: labelsText(shown?.labels),
      a: left,
      b: right,
      target,
      change: changeOf(realismOutcome(left), realismOutcome(right)),
    };
  });
}

/** The key of a scenario, as in the metric view `scenario`. */
export function scenarioKey(scenario: ScenarioTotals): string {
  return `${scenario.application}/${scenario.scenario}`;
}

/** The share of failed sessions of a scenario, or undefined when it attempted none. */
export function failedRatio(scenario: ScenarioTotals): number | undefined {
  return scenario.sessions_attempted > 0
    ? scenario.sessions_failed / scenario.sessions_attempted
    : undefined;
}

/** One scenario in the comparison. A side is undefined when that run has no counts for it. */
export interface ScenarioRow {
  key: string;
  a: ScenarioTotals | undefined;
  b: ScenarioTotals | undefined;
  /** The change of the failed ratio from A to B in points, or `Only in run A` and `Only in run B`. */
  change: string;
}

/** Pairs the scenarios of two runs by application and scenario name, in that order. */
export function pairScenarios(
  a: readonly ScenarioTotals[],
  b: readonly ScenarioTotals[],
): ScenarioRow[] {
  const keys = [...new Set([...a.map(scenarioKey), ...b.map(scenarioKey)])].sort();
  const ratio: Total = {
    id: "ratio",
    label: "Failed ratio",
    value: undefined,
    format: formatPercent,
    formatDelta: formatPoints,
  };
  return keys.map((key) => {
    const left = a.find((scenario) => scenarioKey(scenario) === key);
    const right = b.find((scenario) => scenarioKey(scenario) === key);
    const change =
      left === undefined
        ? CHANGE_WORD["only-b"]
        : right === undefined
          ? CHANGE_WORD["only-a"]
          : totalChange(ratio, failedRatio(left), failedRatio(right)).delta;
    return { key, a: left, b: right, change };
  });
}

/** The run duration in milliseconds, or undefined while the run has no start or no end. */
export function durationMs(run: Run): number | undefined {
  if (run.started_at === undefined || run.ended_at === undefined) return undefined;
  const ms = Date.parse(run.ended_at) - Date.parse(run.started_at);
  return Number.isNaN(ms) ? undefined : ms;
}

/** The whole comparison of two reports. */
export interface Comparison {
  criteria: CriterionRow[];
  realism: RealismRow[];
  /** The scenarios of both runs, or undefined when a report lacks them (a controller older than 0.9.0). */
  scenarios: ScenarioRow[] | undefined;
  /** The criteria and realism rows that went from passing to failing. */
  regressions: number;
}

/** Compares two reports. */
export function compareReports(a: RunReport, b: RunReport): Comparison {
  const criteria = pairCriteria(a.run.verdict?.criteria ?? [], b.run.verdict?.criteria ?? []);
  const realism = pairRealism(a.realism, b.realism);
  const scenarios =
    a.scenarios === undefined || b.scenarios === undefined
      ? undefined
      : pairScenarios(a.scenarios, b.scenarios);
  const regressions = [...criteria, ...realism].filter((row) => row.change === "regressed").length;
  return { criteria, realism, scenarios, regressions };
}
