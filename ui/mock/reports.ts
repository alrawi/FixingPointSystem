// SPDX-License-Identifier: GPL-3.0-only
//
// The report operation of the mock server. The verdict follows the rules of the
// controller: a criterion that compares a value carries `observed` and `passed`,
// and a criterion whose metric has no value carries neither. The mock knows one
// metric, the failed ratio of the whole run, which it takes from the run summary,
// so a criterion on any other metric, such as a mix error, has no value. The
// realism results are fixed, except that a run with failed sessions also drifts in
// its response bytes, so one realism property fails.

import type { CriterionResult, RealismResult, RunSummary, TestConfig } from "../src/api/client";

type Criterion = NonNullable<TestConfig["criteria"]>[number];

/** The whole run value of a metric, or undefined when the mock has none. */
function observedValue(metric: string, summary: RunSummary | undefined): number | undefined {
  switch (metric) {
    case "sessions.failed_ratio": {
      const attempted = summary?.sessions_attempted ?? 0;
      return attempted > 0 ? (summary?.sessions_failed ?? 0) / attempted : undefined;
    }
    default:
      return undefined;
  }
}

function holds(observed: number, op: Criterion["op"], value: number): boolean {
  switch (op) {
    case "<":
      return observed < value;
    case "<=":
      return observed <= value;
    case ">":
      return observed > value;
    case ">=":
      return observed >= value;
    case "==":
      return observed === value;
    case "!=":
      return observed !== value;
  }
}

/** Evaluates the criteria of a configuration against the summary of a finished run. */
export function evaluateCriteria(
  criteria: readonly Criterion[],
  summary: RunSummary | undefined,
): CriterionResult[] {
  return criteria.map((criterion) => {
    const base = { metric: criterion.metric, op: criterion.op, value: criterion.value };
    const observed = observedValue(criterion.metric, summary);
    if (observed === undefined) {
      return { ...base, reason: `metric ${criterion.metric} has no value` };
    }
    const passed = holds(observed, criterion.op, criterion.value);
    return {
      ...base,
      observed,
      passed,
      reason: `observed ${String(observed)} ${criterion.op} ${String(criterion.value)} ${
        passed ? "holds" : "does not hold"
      }`,
    };
  });
}

/** The reason that the controller gives a run whose criteria failed. */
export function failureReason(results: readonly CriterionResult[]): string {
  const failed = results
    .filter((result) => result.passed === false)
    .map((result) => `${result.metric ?? ""}: ${result.reason ?? ""}`);
  return `criteria failed: ${failed.join("; ")}`;
}

/** The realism results of a finished run. */
export function realismResults(hasFailures: boolean): RealismResult[] {
  return [
    { property: "inter_arrival.ks_statistic", observed: 0.03, target: "<= 0.05", passed: true },
    { property: "request_bytes.ks_statistic", observed: 0.02, target: "<= 0.05", passed: true },
    {
      property: "response_bytes.ks_statistic",
      observed: hasFailures ? 0.08 : 0.04,
      target: "<= 0.05",
      passed: !hasFailures,
    },
  ];
}
