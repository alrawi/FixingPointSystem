// SPDX-License-Identifier: GPL-3.0-only

import { Link, useParams } from "react-router";
import "../runs/runs.css";
import "../reports/reports.css";
import {
  ApiError,
  isFinalRunState,
  type CriterionResult,
  type RealismResult,
  type Run,
} from "../api/client";
import {
  criterionLabel,
  criterionOutcome,
  durationMs,
  totalsOf,
  verdictOutcome,
  verdictWord,
} from "../reports/compare";
import { OutcomeText } from "../reports/OutcomeText";
import { formatDuration, formatMetric, formatTimestamp } from "../runs/format";
import { useSignOutWhenUnauthorized } from "./modulesQuery";
import { useRunReport } from "./reportQuery";
import { StateBadge } from "./StateBadge";
import { useHeadingFocus } from "./useHeadingFocus";

/** The report of one run: verdict, criteria, realism results, and totals. */
export function ReportPage() {
  const { id = "" } = useParams();
  const heading = useHeadingFocus<HTMLHeadingElement>();
  const report = useRunReport(id);
  const unauthorized = useSignOutWhenUnauthorized(report.error);
  return (
    <>
      <h1 ref={heading} tabIndex={-1}>
        {`Report of run ${id}`}
      </h1>
      <p>
        <Link to="/runs">All runs</Link> |{" "}
        <Link to={`/runs/${encodeURIComponent(id)}`}>Live view</Link> |{" "}
        <Link to={`/compare?a=${encodeURIComponent(id)}`}>Compare with another run</Link>
      </p>
      {report.isPending && (
        <p role="status" aria-live="polite">
          Loading the report
        </p>
      )}
      {report.isError && !unauthorized && (
        <p role="alert" className="notice">
          {report.error instanceof ApiError && report.error.status === 404
            ? "No run has this id."
            : `The report could not be loaded. ${report.error.message}`}
        </p>
      )}
      {report.data && (
        <>
          <VerdictSection run={report.data.run} />
          <RunFacts run={report.data.run} />
          <CriteriaSection run={report.data.run} />
          <RealismSection results={report.data.realism} run={report.data.run} />
          <TotalsSection run={report.data.run} />
        </>
      )}
    </>
  );
}

function VerdictSection({ run }: { run: Run }) {
  const outcome = verdictOutcome(run);
  return (
    <section aria-labelledby="verdict-heading">
      <h2 id="verdict-heading">Verdict</h2>
      <p className={`verdict outcome-${outcome}`}>{verdictWord(outcome)}</p>
      {!isFinalRunState(run.state) && (
        <p role="status">The run has not ended, so the criteria have no result yet.</p>
      )}
      {isFinalRunState(run.state) && outcome === "none" && (
        <p>The controller gave this run no verdict.</p>
      )}
      {run.reason !== undefined && (
        <p>
          <strong>Reason:</strong> {run.reason}
        </p>
      )}
      {run.degraded === true && (
        <p className="notice">An agent failed during the run, so the results are degraded.</p>
      )}
    </section>
  );
}

function RunFacts({ run }: { run: Run }) {
  const modules = Object.entries(run.module_versions ?? {});
  return (
    <section aria-labelledby="facts-heading">
      <h2 id="facts-heading">Run</h2>
      <dl className="facts">
        <dt>State</dt>
        <dd>
          <StateBadge state={run.state} />
        </dd>
        <dt>Configuration</dt>
        <dd>
          <Link to={`/configs/${encodeURIComponent(run.config_id)}`}>{run.config_id}</Link>
        </dd>
        <dt>Created (UTC)</dt>
        <dd>{formatTimestamp(run.created_at)}</dd>
        {run.started_at !== undefined && (
          <>
            <dt>Started (UTC)</dt>
            <dd>{formatTimestamp(run.started_at)}</dd>
          </>
        )}
        {run.ended_at !== undefined && (
          <>
            <dt>Ended (UTC)</dt>
            <dd>{formatTimestamp(run.ended_at)}</dd>
          </>
        )}
        {durationMs(run) !== undefined && (
          <>
            <dt>Duration</dt>
            <dd>{formatDuration(durationMs(run))}</dd>
          </>
        )}
        {run.seed !== undefined && (
          <>
            <dt>Seed</dt>
            <dd>{String(run.seed)}</dd>
          </>
        )}
        {run.engine_version !== undefined && (
          <>
            <dt>Engine version</dt>
            <dd>{run.engine_version}</dd>
          </>
        )}
        {modules.length > 0 && (
          <>
            <dt>Modules</dt>
            <dd>{modules.map(([name, version]) => `${name} ${version}`).join(", ")}</dd>
          </>
        )}
      </dl>
    </section>
  );
}

function CriteriaSection({ run }: { run: Run }) {
  const criteria: CriterionResult[] = run.verdict?.criteria ?? [];
  return (
    <section aria-labelledby="criteria-heading">
      <h2 id="criteria-heading">Criteria</h2>
      {criteria.length === 0 ? (
        <p>
          {run.verdict === undefined
            ? "No criterion has a result yet."
            : "The configuration of this run has no criteria."}
        </p>
      ) : (
        <table aria-label="Criteria">
          <thead>
            <tr>
              <th scope="col">Criterion</th>
              <th scope="col">Observed</th>
              <th scope="col">Result</th>
              <th scope="col">Reason</th>
            </tr>
          </thead>
          <tbody>
            {criteria.map((result, index) => (
              <tr key={`${criterionLabel(result)}#${String(index)}`}>
                <th scope="row">{criterionLabel(result)}</th>
                <td className="num">{formatMetric(result.observed)}</td>
                <td>
                  <OutcomeText outcome={criterionOutcome(result)} />
                </td>
                <td>{result.reason ?? ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function RealismSection({ results, run }: { results: RealismResult[]; run: Run }) {
  return (
    <section aria-labelledby="realism-heading">
      <h2 id="realism-heading">Realism</h2>
      {results.length === 0 ? (
        <p>
          {isFinalRunState(run.state)
            ? "The report holds no realism results for this run."
            : "No realism result exists yet."}
        </p>
      ) : (
        <table aria-label="Realism results">
          <thead>
            <tr>
              <th scope="col">Property</th>
              <th scope="col">Observed</th>
              <th scope="col">Target</th>
              <th scope="col">Result</th>
            </tr>
          </thead>
          <tbody>
            {results.map((result) => (
              <tr key={result.property}>
                <th scope="row">{result.property}</th>
                <td className="num">{formatMetric(result.observed)}</td>
                <td>{result.target}</td>
                <td>
                  <OutcomeText
                    outcome={result.passed ? "pass" : "fail"}
                    word={result.passed ? "Passed" : "Failed"}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function TotalsSection({ run }: { run: Run }) {
  const totals = totalsOf(run);
  return (
    <section aria-labelledby="totals-heading">
      <h2 id="totals-heading">Totals</h2>
      {run.summary === undefined ? (
        <p>The run has no totals yet.</p>
      ) : (
        <dl className="counters">
          {totals.map((total) => (
            <div key={total.id} className="counter">
              <dt>{total.label}</dt>
              <dd>{total.format(total.value)}</dd>
            </div>
          ))}
        </dl>
      )}
    </section>
  );
}
