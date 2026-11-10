// SPDX-License-Identifier: GPL-3.0-only
//
// The run and metrics operations of the mock server. A run follows a fixed
// script on a clock that tests can control: pending, preparing, running for a
// set number of ticks, then completed. A stop request moves it through
// stopping to aborted, and a forced stop aborts it at once. Every metric value
// is a function of the tick number, so a test can compute the expected numbers.
// Sample timestamps derive from the tick number as well, never from the clock.

import type { Agent, MetricSample, MetricView, Run, RunState, RunSummary } from "../src/api/client";
import type { StoredConfig } from "./fixtures";

/** The time of tick 0. Tick t carries the timestamp that lies t seconds later. */
export const MOCK_RUN_EPOCH_MS = Date.UTC(2026, 10, 1, 10, 0, 0);

/** The session rate of the whole mix once the ramp is over. */
const TOTAL_RATE = 100;
/** The number of ticks in which the rate ramps up. */
const RAMP_TICKS = 5;

/** A run that the mock holds. */
interface MockRun {
  id: string;
  configId: string;
  /** The tick in which the run was created. */
  createdTick: number;
  /** The key and the share of each application of the configuration. */
  apps: { key: string; scenario: string; rate: number }[];
  /** The tick of a stop request and whether it was forced. */
  stop?: { tick: number; force: boolean };
  /** How many ticks the run lasts when nobody stops it. */
  duration: number;
}

export interface MockRunOptions {
  /** The clock in milliseconds. Tests pass a manual one. */
  now?: () => number;
  /** The length of one tick in milliseconds, one second as in the contract. */
  stepMs?: number;
  /** How many ticks a run keeps running when nobody stops it. */
  runTicks?: number;
}

/** The values of one application at a tick, keyed by the names of the metric catalog. */
export function applicationValues(app: { rate: number }, tick: number): Record<string, number> {
  let attempted = 0;
  for (let s = 1; s <= tick; s += 1) attempted += rateAt(app.rate, s);
  const rate = rateAt(app.rate, tick);
  const concurrent = Math.round(rate / 2);
  const failed = Math.floor(attempted / 50);
  const p50 = 12_000 + 100 * (tick % 5) + 10 * app.rate;
  const duration50 = 3 * p50;
  return {
    "sessions.attempted": attempted,
    "sessions.succeeded": attempted - failed - concurrent,
    "sessions.failed": failed,
    "sessions.concurrent": concurrent,
    "sessions.rate": rate,
    "sessions.response_time.p50": p50,
    "sessions.response_time.p90": 2 * p50,
    "sessions.response_time.p99": 4 * p50,
    "sessions.duration.p50": duration50,
    "sessions.duration.p90": 2 * duration50,
    "sessions.duration.p99": 4 * duration50,
    "bytes.tx": attempted * 2048,
    "bytes.rx": attempted * 8192,
    "throughput.bps": rate * 8 * 4096,
  };
}

function rateAt(rate: number, tick: number): number {
  return Math.round((rate * Math.min(tick, RAMP_TICKS)) / RAMP_TICKS);
}

/** The timestamp of a tick. */
export function tickTime(tick: number): string {
  return new Date(MOCK_RUN_EPOCH_MS + tick * 1000).toISOString();
}

/** The sum of the application values that the run summary reports. */
function summaryAt(apps: MockRun["apps"], tick: number): RunSummary {
  const sum = (name: string) =>
    apps.reduce((total, app) => total + (applicationValues(app, tick)[name] ?? 0), 0);
  return {
    sessions_attempted: sum("sessions.attempted"),
    sessions_succeeded: sum("sessions.succeeded"),
    sessions_failed: sum("sessions.failed"),
    bytes_tx: sum("bytes.tx"),
    bytes_rx: sum("bytes.rx"),
    throughput_bps: sum("throughput.bps"),
    sessions_per_second: sum("sessions.rate"),
    concurrent_sessions: sum("sessions.concurrent"),
  };
}

/** The runs of one mock server. */
export class MockRuns {
  private readonly runs: MockRun[] = [];
  private readonly now: () => number;
  readonly stepMs: number;
  private readonly runTicks: number;
  private readonly origin: number;

  constructor(options: MockRunOptions = {}) {
    this.now = options.now ?? (() => Date.now());
    this.stepMs = options.stepMs ?? 1000;
    this.runTicks = options.runTicks ?? 30;
    this.origin = this.now();
  }

  /** The number of whole ticks since the server started. */
  private clockTick(): number {
    return Math.floor((this.now() - this.origin) / this.stepMs);
  }

  /** Creates a run for a stored configuration. */
  create(config: StoredConfig, moduleOf: (scenario: string) => string): Run {
    const mix = config.body.profile.mix;
    const total = mix.reduce((sum, entry) => sum + entry.weight, 0);
    const byModule = new Map<string, { scenario: string; weight: number }>();
    for (const entry of mix) {
      const key = moduleOf(entry.scenario);
      const known = byModule.get(key);
      byModule.set(key, { scenario: entry.scenario, weight: (known?.weight ?? 0) + entry.weight });
    }
    const apps = [...byModule.entries()]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, value]) => ({
        key,
        scenario: value.scenario,
        rate: Math.max(1, Math.round((TOTAL_RATE * value.weight) / total)),
      }));
    const run: MockRun = {
      id: `run_mock${String(this.runs.length + 1).padStart(4, "0")}`,
      configId: config.id,
      createdTick: this.clockTick(),
      apps,
      duration: this.runTicks,
    };
    this.runs.push(run);
    return this.view(run);
  }

  find(id: string): MockRun | undefined {
    return this.runs.find((run) => run.id === id);
  }

  /** The run state and the newest tick that has samples, from the script. */
  private status(run: MockRun): { state: RunState; lastTick: number } {
    const elapsed = this.clockTick() - run.createdTick;
    // Tick 1 is the first second of traffic, which starts after pending and preparing.
    const traffic = Math.max(0, elapsed - 1);
    let lastTick = Math.min(traffic, run.duration);
    let state: RunState = elapsed < 1 ? "pending" : elapsed < 2 ? "preparing" : "running";
    if (traffic >= run.duration) state = "completed";
    if (run.stop) {
      const stopTraffic = Math.max(0, run.stop.tick - run.createdTick - 1);
      if (stopTraffic < run.duration) {
        const stoppedAt = this.clockTick() - run.stop.tick;
        if (run.stop.force || stoppedAt >= 1) {
          state = "aborted";
          lastTick = Math.min(lastTick, stopTraffic + (run.stop.force ? 0 : 1));
        } else {
          state = "stopping";
          lastTick = Math.min(lastTick, stopTraffic);
        }
      }
    }
    return { state, lastTick };
  }

  /** The Run object of the contract. */
  view(run: MockRun): Run {
    const { state, lastTick } = this.status(run);
    const final = state === "completed" || state === "aborted" || state === "failed";
    const out: Run = {
      id: run.id,
      config_id: run.configId,
      state,
      phase: state === "running" ? "steady" : state,
      created_at: tickTime(-2),
      seed: 4711,
      engine_version: "0.1.0",
    };
    if (state === "running" || state === "stopping" || final) out.started_at = tickTime(0);
    if (final) out.ended_at = tickTime(lastTick);
    if (state === "aborted") out.reason = "stopped by request";
    if (lastTick > 0) out.summary = summaryAt(run.apps, lastTick);
    return out;
  }

  list(state: string | null): Run[] {
    return [...this.runs]
      .reverse()
      .map((run) => this.view(run))
      .filter((run) => state === null || run.state === state);
  }

  /** Records a stop request. Returns the changed run, or undefined when the run already ended. */
  stop(run: MockRun, force: boolean): Run | undefined {
    const { state } = this.status(run);
    if (state === "completed" || state === "aborted" || state === "failed") return undefined;
    run.stop ??= { tick: this.clockTick(), force };
    return this.view(run);
  }

  /** The samples of a tick for the views that the mock fills. */
  samplesAt(run: MockRun, views: readonly MetricView[], tick: number): MetricSample[] {
    const samples: MetricSample[] = [];
    for (const view of views) {
      if (view !== "application" && view !== "scenario") continue;
      for (const app of run.apps) {
        samples.push({
          ts: tickTime(tick),
          view,
          key: view === "application" ? app.key : app.scenario,
          values: applicationValues(app, tick),
        });
      }
    }
    return samples;
  }

  /** The samples of the newest tick, as the stream sends them. */
  latest(run: MockRun, views: readonly MetricView[]): MetricSample[] {
    const { lastTick } = this.status(run);
    return lastTick > 0 ? this.samplesAt(run, views, lastTick) : [];
  }

  /** The samples from a time on, one per tick, as getRunMetrics answers. */
  series(run: MockRun, view: MetricView, from: string | null): MetricSample[] {
    const { lastTick } = this.status(run);
    const start = from === null ? 0 : Date.parse(from);
    const samples: MetricSample[] = [];
    for (let tick = 1; tick <= lastTick; tick += 1) {
      if (MOCK_RUN_EPOCH_MS + tick * 1000 < start) continue;
      samples.push(...this.samplesAt(run, [view], tick));
    }
    return samples;
  }

  /** The state of a run for the end event of the stream. */
  stateOf(run: MockRun): { state: RunState; final: boolean } {
    const { state } = this.status(run);
    return { state, final: state === "completed" || state === "aborted" || state === "failed" };
  }
}

/** The agents that a start request may name. */
export function unknownAgents(requested: string[], known: Agent[]): string[] {
  return requested.filter((id) => !known.some((agent) => agent.id === id));
}
