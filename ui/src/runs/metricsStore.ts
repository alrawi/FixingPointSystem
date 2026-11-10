// SPDX-License-Identifier: GPL-3.0-only
//
// Holds the samples of one run outside React state, as the UI stack rules
// require (ui/docs/README.md, rule 2). The store keeps a ring of the newest
// ticks of the `application` view, merges samples that the stream, the
// history request, and the polling fallback deliver in any order, and tells
// subscribers once per flush, so a stream of one event per second and key does
// not render the page once per event.

import type { MetricSample } from "../api/client";

/** One value of a series at a time. */
export interface Point {
  /** Milliseconds since the epoch. */
  t: number;
  v: number;
}

/** How many ticks the store keeps. At one tick per second this is ten minutes. */
export const MAX_TICKS = 600;

/** The view whose samples the run view charts. It carries the scenario metrics summed over scenarios. */
export const RUN_VIEW = "application";

/** The headline totals of the newest tick, summed over all applications. */
export interface Totals {
  /** Timestamp of the tick. */
  ts: string;
  attempted: number | undefined;
  succeeded: number | undefined;
  failed: number | undefined;
  rate: number | undefined;
  concurrent: number | undefined;
  throughputBps: number | undefined;
  bytesTx: number | undefined;
  bytesRx: number | undefined;
}

type Values = Record<string, number>;

/** Sums a metric over the keys of one tick, or undefined when no key reports it. */
function sumOf(keys: Map<string, Values>, metric: string): number | undefined {
  let total: number | undefined;
  for (const values of keys.values()) {
    const value = values[metric];
    if (value !== undefined) total = (total ?? 0) + value;
  }
  return total;
}

export class MetricsStore {
  /** Samples by tick time and application key. */
  private readonly ticks = new Map<number, Map<string, Values>>();
  /** The tick times in ascending order. */
  private order: number[] = [];
  private readonly listeners = new Set<() => void>();
  private snapshot = 0;
  private dirty = false;

  /** Adds the samples of the `application` view and ignores every other view. */
  add(samples: readonly MetricSample[]): void {
    for (const sample of samples) {
      if (sample.view !== RUN_VIEW) continue;
      const t = Date.parse(sample.ts);
      if (Number.isNaN(t)) continue;
      let keys = this.ticks.get(t);
      if (keys === undefined) {
        keys = new Map();
        this.ticks.set(t, keys);
        this.insert(t);
      }
      keys.set(sample.key, sample.values);
      this.dirty = true;
    }
    while (this.order.length > MAX_TICKS) {
      const oldest = this.order.shift();
      if (oldest !== undefined) this.ticks.delete(oldest);
    }
  }

  private insert(t: number): void {
    const last = this.order[this.order.length - 1];
    if (last === undefined || t > last) {
      this.order.push(t);
      return;
    }
    // A late sample goes to its place. Binary search keeps a long backfill cheap.
    let low = 0;
    let high = this.order.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if ((this.order[mid] ?? 0) < t) low = mid + 1;
      else high = mid;
    }
    this.order.splice(low, 0, t);
  }

  /** The timestamp of the newest tick as ISO text, for the `from` parameter of a backfill. */
  newest(): string | undefined {
    const last = this.order[this.order.length - 1];
    return last === undefined ? undefined : new Date(last).toISOString();
  }

  get size(): number {
    return this.order.length;
  }

  /** The application keys that appeared, sorted. */
  keys(): string[] {
    const keys = new Set<string>();
    for (const tick of this.ticks.values()) for (const key of tick.keys()) keys.add(key);
    return [...keys].sort();
  }

  /** The newest `limit` points of a metric summed over every application. */
  total(metric: string, limit = MAX_TICKS): Point[] {
    return this.series((keys) => sumOf(keys, metric), limit);
  }

  /** The newest `limit` points of a metric of one application. */
  of(key: string, metric: string, limit = MAX_TICKS): Point[] {
    return this.series((keys) => keys.get(key)?.[metric], limit);
  }

  private series(pick: (keys: Map<string, Values>) => number | undefined, limit: number): Point[] {
    const points: Point[] = [];
    for (const t of this.order.slice(-limit)) {
      const keys = this.ticks.get(t);
      const v = keys === undefined ? undefined : pick(keys);
      if (v !== undefined) points.push({ t, v });
    }
    return points;
  }

  /** The totals of the newest tick, or undefined while the store is empty. */
  latest(): Totals | undefined {
    const t = this.order[this.order.length - 1];
    const keys = t === undefined ? undefined : this.ticks.get(t);
    if (t === undefined || keys === undefined) return undefined;
    return {
      ts: new Date(t).toISOString(),
      attempted: sumOf(keys, "sessions.attempted"),
      succeeded: sumOf(keys, "sessions.succeeded"),
      failed: sumOf(keys, "sessions.failed"),
      rate: sumOf(keys, "sessions.rate"),
      concurrent: sumOf(keys, "sessions.concurrent"),
      throughputBps: sumOf(keys, "throughput.bps"),
      bytesTx: sumOf(keys, "bytes.tx"),
      bytesRx: sumOf(keys, "bytes.rx"),
    };
  }

  /** Tells the subscribers when samples arrived since the last flush. */
  flush(): void {
    if (!this.dirty) return;
    this.dirty = false;
    this.snapshot += 1;
    for (const listener of this.listeners) listener();
  }

  /** For useSyncExternalStore. */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** For useSyncExternalStore. The number changes with every flush that had news. */
  getSnapshot = (): number => this.snapshot;
}
