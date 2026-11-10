// SPDX-License-Identifier: GPL-3.0-only

import { useQueryClient } from "@tanstack/react-query";
import "../runs/runs.css";
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";
import { ApiError, isFinalRunState, type Run, type RunState } from "../api/client";
import {
  formatBitsPerSecond,
  formatBytes,
  formatCount,
  formatDecimal,
  formatMicros,
  formatPercent,
  formatTimestamp,
} from "../runs/format";
import { LineChart } from "../runs/LineChart";
import { RUN_VIEW, type MetricsStore, type Totals } from "../runs/metricsStore";
import { useRunMetrics, type FeedStatus } from "../runs/useRunMetrics";
import { useSignOutWhenUnauthorized } from "./modulesQuery";
import { runKey, useRun, useStopRun } from "./runsQuery";
import { StateBadge } from "./StateBadge";
import { useHeadingFocus } from "./useHeadingFocus";

/** The live view of one run. The run id keys the view, so a new id starts with an empty store. */
export function RunPage() {
  const { id = "" } = useParams();
  return <RunView key={id} id={id} />;
}

const FEED_TEXT: Record<FeedStatus, string> = {
  connecting: "Connecting to the metrics stream.",
  live: "Live. One sample arrives every second.",
  reconnecting: "The connection dropped. Reconnecting and filling the gap.",
  polling: "The controller has no metrics stream, so the view polls once per second.",
  ended: "The run ended and the metrics are complete.",
  failed: "The metrics are not available.",
};

/** The states in which a stop request still has an effect. */
function canStop(state: RunState): boolean {
  return state === "pending" || state === "preparing" || state === "running";
}

function RunView({ id }: { id: string }) {
  const heading = useHeadingFocus<HTMLHeadingElement>();
  const queryClient = useQueryClient();
  const run = useRun(id);
  const unauthorized = useSignOutWhenUnauthorized(run.error);
  const metrics = useRunMetrics(id, run.data);
  const history = useStateHistory(run.data?.state);
  const { status } = metrics;

  // The end event may arrive before the next poll, so refresh the run at once.
  useEffect(() => {
    if (status === "ended") void queryClient.invalidateQueries({ queryKey: runKey(id) });
  }, [status, queryClient, id]);

  return (
    <>
      <h1 ref={heading} tabIndex={-1}>
        {`Run ${id}`}
      </h1>
      <p>
        <Link to="/runs">All runs</Link>
      </p>
      {run.isPending && (
        <p role="status" aria-live="polite">
          Loading the run
        </p>
      )}
      {run.isError && !unauthorized && (
        <p role="alert" className="notice">
          {run.error instanceof ApiError && run.error.status === 404
            ? "No run has this id."
            : `The run could not be loaded. ${run.error.message}`}
        </p>
      )}
      {run.data && (
        <>
          <RunHeader run={run.data} history={history} />
          <p role="status" aria-live="polite" className="feed-status">
            {FEED_TEXT[status]}
            {metrics.error !== undefined && ` ${metrics.error}`}
          </p>
          <Counters run={run.data} store={metrics.store} />
          <Charts store={metrics.store} />
        </>
      )}
    </>
  );
}

/** The states that the view saw, oldest first. The API has no transition log, so the view keeps its own. */
function useStateHistory(state: RunState | undefined): RunState[] {
  const [history, setHistory] = useState<RunState[]>([]);
  // Derived from the polled state during render, guarded so it settles after one pass.
  if (state !== undefined && history[history.length - 1] !== state) {
    setHistory([...history, state]);
  }
  return history;
}

function RunHeader({ run, history }: { run: Run; history: RunState[] }) {
  const stop = useStopRun(run.id);
  const [force, setForce] = useState(false);
  return (
    <section aria-labelledby="state-heading">
      <h2 id="state-heading">State</h2>
      <p>
        <StateBadge state={run.state} />
        {run.degraded === true && (
          <span className="notice">
            {" "}
            An agent failed during the run, so the results are degraded.
          </span>
        )}
      </p>
      {run.reason !== undefined && (
        <p role="status">
          <strong>Reason:</strong> {run.reason}
        </p>
      )}
      <ol aria-label="State transitions" className="transitions">
        {history.map((state, index) => (
          <li key={`${String(index)}-${state}`}>
            {state}
            {index === history.length - 1 && !isFinalRunState(state) ? " (current)" : ""}
            {index === history.length - 1 && isFinalRunState(state) ? " (final)" : ""}
          </li>
        ))}
      </ol>
      <dl className="facts">
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
      </dl>
      {canStop(run.state) && (
        <div className="editor-actions">
          <button
            type="button"
            disabled={stop.isPending}
            onClick={() => {
              stop.mutate(force);
            }}
          >
            Stop run
          </button>
          <label>
            <input
              type="checkbox"
              checked={force}
              onChange={(event) => {
                setForce(event.target.checked);
              }}
            />{" "}
            Force: abort at once and skip ramp down
          </label>
        </div>
      )}
      {run.state === "stopping" && (
        <p role="status" aria-live="polite">
          The run is stopping.
        </p>
      )}
      {stop.isError && (
        <p role="alert" className="notice">
          The run was not stopped. {stop.error instanceof ApiError ? stop.error.message : ""}
        </p>
      )}
    </section>
  );
}

/** The totals of the newest tick, or the summary of the run until the first sample arrives. */
function currentTotals(run: Run, store: MetricsStore): Partial<Totals> {
  const latest = store.latest();
  if (latest !== undefined) return latest;
  const summary = run.summary;
  if (summary === undefined) return {};
  return {
    attempted: summary.sessions_attempted,
    succeeded: summary.sessions_succeeded,
    failed: summary.sessions_failed,
    rate: summary.sessions_per_second,
    concurrent: summary.concurrent_sessions,
    throughputBps: summary.throughput_bps,
    bytesTx: summary.bytes_tx,
    bytesRx: summary.bytes_rx,
  };
}

function Counters({ run, store }: { run: Run; store: MetricsStore }) {
  const totals = currentTotals(run, store);
  const ratio =
    totals.attempted !== undefined && totals.failed !== undefined && totals.attempted > 0
      ? totals.failed / totals.attempted
      : undefined;
  const items: [string, string][] = [
    ["Sessions attempted", formatCount(totals.attempted)],
    ["Sessions succeeded", formatCount(totals.succeeded)],
    ["Sessions failed", formatCount(totals.failed)],
    ["Failed ratio", formatPercent(ratio)],
    ["Sessions per second", formatDecimal(totals.rate)],
    ["Concurrent sessions", formatCount(totals.concurrent)],
    ["Throughput", formatBitsPerSecond(totals.throughputBps)],
    ["Bytes sent", formatBytes(totals.bytesTx)],
    ["Bytes received", formatBytes(totals.bytesRx)],
  ];
  return (
    <section aria-labelledby="counters-heading">
      <h2 id="counters-heading">Counters</h2>
      <dl className="counters">
        {items.map(([label, value]) => (
          <div key={label} className="counter">
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function Charts({ store }: { store: MetricsStore }) {
  const [chosen, setChosen] = useState("");
  const keys = store.keys();
  const key = keys.includes(chosen) ? chosen : (keys[0] ?? "");
  const latency = (prefix: string) =>
    (["p50", "p90", "p99"] as const).map((stat) => ({
      id: `${prefix}.${stat}`,
      label: stat,
      points: store.of(key, `${prefix}.${stat}`),
    }));
  return (
    <section aria-labelledby="charts-heading">
      <h2 id="charts-heading">Charts</h2>
      <p className="hint">{`Samples of the ${RUN_VIEW} view, client side. Counters and rates add up over applications. Latencies belong to one application.`}</p>
      {keys.length > 1 && (
        <div className="field">
          <label htmlFor="latency-app">Application for the latency charts</label>
          <select
            id="latency-app"
            value={key}
            onChange={(event) => {
              setChosen(event.target.value);
            }}
          >
            {keys.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </div>
      )}
      <div className="charts">
        <LineChart
          title="Response time"
          series={latency("sessions.response_time")}
          format={formatMicros}
        />
        <LineChart
          title="Session duration"
          series={latency("sessions.duration")}
          format={formatMicros}
        />
        <LineChart
          title="Sessions per second"
          series={[{ id: "rate", label: "rate", points: store.total("sessions.rate") }]}
          format={formatDecimal}
        />
        <LineChart
          title="Concurrent sessions"
          series={[{ id: "concurrent", label: "open", points: store.total("sessions.concurrent") }]}
          format={formatCount}
        />
        <LineChart
          title="Throughput"
          series={[{ id: "bps", label: "payload", points: store.total("throughput.bps") }]}
          format={formatBitsPerSecond}
        />
      </div>
    </section>
  );
}
