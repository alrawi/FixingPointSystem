// SPDX-License-Identifier: GPL-3.0-only

import type { ReactNode } from "react";
import { Link, useSearchParams } from "react-router";
import "../runs/runs.css";
import "../reports/reports.css";
import { ApiError, type CriterionResult, type Run, type RunReport } from "../api/client";
import {
  CHANGE_WORD,
  compareReports,
  criterionOutcome,
  criterionWord,
  durationMs,
  totalChange,
  totalsOf,
  verdictOutcome,
  verdictWord,
  type Change,
  type Comparison,
} from "../reports/compare";
import { OutcomeText } from "../reports/OutcomeText";
import { formatDuration, formatMetric } from "../runs/format";
import { useSignOutWhenUnauthorized } from "./modulesQuery";
import { useRunReport } from "./reportQuery";
import { useRuns } from "./runsQuery";
import { StateBadge } from "./StateBadge";
import { useHeadingFocus } from "./useHeadingFocus";

/** Compares the reports of two runs side by side. The runs come from the query string. */
export function ComparePage() {
  const heading = useHeadingFocus<HTMLHeadingElement>();
  const [params, setParams] = useSearchParams();
  const a = params.get("a") ?? "";
  const b = params.get("b") ?? "";
  const runs = useRuns();
  const unauthorized = useSignOutWhenUnauthorized(runs.error);

  function choose(side: "a" | "b", id: string) {
    const next = new URLSearchParams(params);
    if (id === "") next.delete(side);
    else next.set(side, id);
    setParams(next);
  }

  return (
    <>
      <h1 ref={heading} tabIndex={-1}>
        Compare runs
      </h1>
      <p>
        <Link to="/runs">All runs</Link>
      </p>
      {runs.isError && !unauthorized && (
        <p role="alert" className="notice">
          Runs could not be loaded. {runs.error.message}
        </p>
      )}
      <form
        className="compare-form"
        onSubmit={(event) => {
          event.preventDefault();
        }}
      >
        <RunSelect
          label="Run A"
          value={a}
          runs={runs.data}
          loading={runs.isPending}
          onChange={(id) => {
            choose("a", id);
          }}
        />
        <RunSelect
          label="Run B"
          value={b}
          runs={runs.data}
          loading={runs.isPending}
          onChange={(id) => {
            choose("b", id);
          }}
        />
      </form>
      {a === "" || b === "" ? (
        <p>Choose two runs to compare their reports.</p>
      ) : a === b ? (
        <p role="alert" className="notice">
          Choose two different runs.
        </p>
      ) : (
        <ReportComparison key={`${a}|${b}`} a={a} b={b} />
      )}
    </>
  );
}

function RunSelect({
  label,
  value,
  runs,
  loading,
  onChange,
}: {
  label: string;
  value: string;
  runs: Run[] | undefined;
  loading: boolean;
  onChange: (id: string) => void;
}) {
  const id = `compare-${label.replace(" ", "-").toLowerCase()}`;
  const known = runs?.some((run) => run.id === value) === true;
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      <select
        id={id}
        value={value}
        disabled={loading}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      >
        <option value="">Choose a run</option>
        {/* A run in the link that the list lacks stays selectable, so the link does not lose it. */}
        {value !== "" && !known && <option value={value}>{value}</option>}
        {(runs ?? []).map((run) => (
          <option key={run.id} value={run.id}>
            {`${run.id} (${run.state})`}
          </option>
        ))}
      </select>
    </div>
  );
}

function ReportComparison({ a, b }: { a: string; b: string }) {
  const left = useRunReport(a);
  const right = useRunReport(b);
  const unauthorized = useSignOutWhenUnauthorized(left.error ?? right.error);
  const failures = [
    { side: "A", id: a, query: left },
    { side: "B", id: b, query: right },
  ].filter((entry) => entry.query.isError);
  if (failures.length === 0 && (left.isPending || right.isPending)) {
    return (
      <p role="status" aria-live="polite">
        Loading the reports
      </p>
    );
  }
  if (failures.length > 0) {
    if (unauthorized) return null;
    return (
      <>
        {failures.map(({ side, id, query }) => (
          <p key={side} role="alert" className="notice">
            {query.error instanceof ApiError && query.error.status === 404
              ? `Run ${side} (${id}): no run has this id.`
              : `The report of run ${side} (${id}) could not be loaded. ${query.error?.message ?? ""}`}
          </p>
        ))}
      </>
    );
  }
  if (left.data === undefined || right.data === undefined) return null;
  return <ComparisonTables a={left.data} b={right.data} />;
}

function ComparisonTables({ a, b }: { a: RunReport; b: RunReport }) {
  const comparison = compareReports(a, b);
  const active = [a.run, b.run].filter((run) => verdictOutcome(run) === "none");
  return (
    <>
      <section aria-labelledby="summary-heading">
        <h2 id="summary-heading">Summary</h2>
        <p role="status">{summaryText(comparison)}</p>
        {active.length > 0 && (
          <p>A run that has not ended has no verdict, so its criteria show no result.</p>
        )}
      </section>
      <FactsTable a={a.run} b={b.run} />
      <TotalsTable a={a.run} b={b.run} />
      <CriteriaTable comparison={comparison} />
      <RealismTable comparison={comparison} />
    </>
  );
}

function summaryText(comparison: Comparison): string {
  const count = comparison.regressions;
  if (count === 0) return "No criterion or realism result got worse from run A to run B.";
  return count === 1
    ? "1 criterion or realism result got worse from run A to run B."
    : `${String(count)} criteria or realism results got worse from run A to run B.`;
}

function RunHeads({ a, b }: { a: Run; b: Run }) {
  return (
    <>
      <th scope="col">
        Run A: <Link to={`/runs/${encodeURIComponent(a.id)}/report`}>{a.id}</Link>
      </th>
      <th scope="col">
        Run B: <Link to={`/runs/${encodeURIComponent(b.id)}/report`}>{b.id}</Link>
      </th>
    </>
  );
}

function FactsTable({ a, b }: { a: Run; b: Run }) {
  const rows: [string, ReactNode, ReactNode][] = [
    ["State", <StateBadge key="a" state={a.state} />, <StateBadge key="b" state={b.state} />],
    [
      "Verdict",
      <OutcomeText key="a" outcome={verdictOutcome(a)} word={verdictWord(verdictOutcome(a))} />,
      <OutcomeText key="b" outcome={verdictOutcome(b)} word={verdictWord(verdictOutcome(b))} />,
    ],
    ["Configuration", a.config_id, b.config_id],
    [
      "Seed",
      a.seed === undefined ? "-" : String(a.seed),
      b.seed === undefined ? "-" : String(b.seed),
    ],
    ["Engine version", a.engine_version ?? "-", b.engine_version ?? "-"],
    ["Duration", formatDuration(durationMs(a)), formatDuration(durationMs(b))],
  ];
  return (
    <section aria-labelledby="facts-heading">
      <h2 id="facts-heading">Runs</h2>
      <table aria-label="Runs compared" className="compare-table">
        <thead>
          <tr>
            <th scope="col">Property</th>
            <RunHeads a={a} b={b} />
          </tr>
        </thead>
        <tbody>
          {rows.map(([label, left, right]) => (
            <tr key={label}>
              <th scope="row">{label}</th>
              <td>{left}</td>
              <td>{right}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

function TotalsTable({ a, b }: { a: Run; b: Run }) {
  const left = totalsOf(a);
  const right = totalsOf(b);
  return (
    <section aria-labelledby="totals-heading">
      <h2 id="totals-heading">Totals</h2>
      <table aria-label="Totals compared" className="compare-table">
        <thead>
          <tr>
            <th scope="col">Total</th>
            <RunHeads a={a} b={b} />
            <th scope="col">Change from A to B</th>
          </tr>
        </thead>
        <tbody>
          {left.map((total, index) => {
            const other = right[index];
            const change = totalChange(total, total.value, other?.value);
            return (
              <tr key={total.id}>
                <th scope="row">{total.label}</th>
                <td className="num">{total.format(total.value)}</td>
                <td className="num">{total.format(other?.value)}</td>
                <td className="num">
                  {change.delta}
                  {change.percent !== "" && ` (${change.percent})`}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </section>
  );
}

function ChangeText({ change }: { change: Change }) {
  return <span className={`outcome-word change-${change}`}>{CHANGE_WORD[change]}</span>;
}

function criterionCell(result: CriterionResult | undefined) {
  if (result === undefined) return "Not in this run";
  const observed =
    result.observed === undefined ? "" : ` (observed ${formatMetric(result.observed)})`;
  return (
    <>
      <OutcomeText
        outcome={criterionOutcome(result)}
        word={criterionWord(criterionOutcome(result))}
      />
      {observed}
    </>
  );
}

function CriteriaTable({ comparison }: { comparison: Comparison }) {
  return (
    <section aria-labelledby="criteria-heading">
      <h2 id="criteria-heading">Criteria</h2>
      {comparison.criteria.length === 0 ? (
        <p>Neither run has criteria results.</p>
      ) : (
        <table aria-label="Criteria compared" className="compare-table">
          <thead>
            <tr>
              <th scope="col">Criterion</th>
              <th scope="col">Run A</th>
              <th scope="col">Run B</th>
              <th scope="col">Change</th>
            </tr>
          </thead>
          <tbody>
            {comparison.criteria.map((row) => (
              <tr key={row.key}>
                <th scope="row">{row.label}</th>
                <td>{criterionCell(row.a)}</td>
                <td>{criterionCell(row.b)}</td>
                <td>
                  <ChangeText change={row.change} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function RealismTable({ comparison }: { comparison: Comparison }) {
  const cell = (result: { observed: number; passed: boolean } | undefined) =>
    result === undefined ? (
      "Not in this run"
    ) : (
      <>
        <OutcomeText
          outcome={result.passed ? "pass" : "fail"}
          word={result.passed ? "Passed" : "Failed"}
        />
        {` (observed ${formatMetric(result.observed)})`}
      </>
    );
  return (
    <section aria-labelledby="realism-heading">
      <h2 id="realism-heading">Realism</h2>
      {comparison.realism.length === 0 ? (
        <p>Neither run has realism results.</p>
      ) : (
        <table aria-label="Realism compared" className="compare-table">
          <thead>
            <tr>
              <th scope="col">Property</th>
              <th scope="col">Target</th>
              <th scope="col">Run A</th>
              <th scope="col">Run B</th>
              <th scope="col">Change</th>
            </tr>
          </thead>
          <tbody>
            {comparison.realism.map((row) => (
              <tr key={row.property}>
                <th scope="row">{row.property}</th>
                <td>{row.target}</td>
                <td>{cell(row.a)}</td>
                <td>{cell(row.b)}</td>
                <td>
                  <ChangeText change={row.change} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
