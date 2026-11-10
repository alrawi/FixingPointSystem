// SPDX-License-Identifier: GPL-3.0-only
//
// The report operation of the mock server. The verdict follows the rules of the
// controller: a criterion that compares a value carries `observed` and `passed`,
// and a criterion whose metric has no value carries neither. The mock knows one
// metric, the failed ratio of the whole run, which it takes from the run summary,
// so a criterion on any other metric, such as a mix error, has no value. The
// realism results are fixed, except that a run with failed sessions also drifts in
// its response bytes, so one realism property fails.

import type {
  CriterionResult,
  RealismResult,
  RunSummary,
  ScenarioTotals,
  TestConfig,
} from "../src/api/client";

type Criterion = NonNullable<TestConfig["criteria"]>[number];

/**
 * The value of a metric over the series that the filter selects, or undefined when the mock has
 * none. Without a filter the series are the whole run. The filter `application` selects the
 * scenarios of one application, and any other label has no series in the mock.
 */
function observedValue(
  metric: string,
  where: Readonly<Record<string, string>> | undefined,
  summary: RunSummary | undefined,
  scenarios: readonly ScenarioTotals[],
): number | undefined {
  const labels = Object.entries(where ?? {});
  const application = labels.find(([name]) => name === "application")?.[1];
  if (labels.some(([name]) => name !== "application")) return undefined;
  switch (metric) {
    case "sessions.failed_ratio": {
      if (application !== undefined) {
        const own = scenarios.filter((scenario) => scenario.application === application);
        const attempted = own.reduce((sum, scenario) => sum + scenario.sessions_attempted, 0);
        const failed = own.reduce((sum, scenario) => sum + scenario.sessions_failed, 0);
        return attempted > 0 ? failed / attempted : undefined;
      }
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
  scenarios: readonly ScenarioTotals[] = [],
): CriterionResult[] {
  return criteria.map((criterion) => {
    const base: CriterionResult = {
      metric: criterion.metric,
      op: criterion.op,
      value: criterion.value,
      ...(criterion.where === undefined ? {} : { where: criterion.where }),
    };
    const observed = observedValue(criterion.metric, criterion.where, summary, scenarios);
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

/**
 * The realism results of a finished run, named as the controller names them: the metric and the
 * labels of the series, and a property in words.
 */
export function realismResults(hasFailures: boolean): RealismResult[] {
  const ks = (property: string, observed: number, passed: boolean): RealismResult => ({
    property: `${property}.ks_statistic`,
    metric: "realism.ks_statistic",
    labels: { application: "all", property },
    observed,
    target: "<= 0.05",
    passed,
  });
  return [
    ks("inter_arrival", 0.03, true),
    ks("request_bytes", 0.02, true),
    ks("response_bytes", hasFailures ? 0.08 : 0.04, !hasFailures),
  ];
}
