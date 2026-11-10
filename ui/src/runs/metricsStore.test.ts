// SPDX-License-Identifier: GPL-3.0-only

import { describe, expect, it, vi } from "vitest";
import type { MetricSample } from "../api/client";
import { MAX_TICKS, MetricsStore } from "./metricsStore";

const T0 = Date.UTC(2026, 10, 1, 10, 0, 0);

function sample(tick: number, key: string, values: Record<string, number>): MetricSample {
  return { ts: new Date(T0 + tick * 1000).toISOString(), view: "application", key, values };
}

describe("MetricsStore", () => {
  it("sums counters over applications and keeps latencies per application", () => {
    const store = new MetricsStore();
    store.add([
      sample(1, "http1", { "sessions.attempted": 10, "sessions.response_time.p50": 100 }),
      sample(1, "dns", { "sessions.attempted": 5, "sessions.response_time.p50": 7 }),
      sample(2, "http1", { "sessions.attempted": 25, "sessions.response_time.p50": 110 }),
      sample(2, "dns", { "sessions.attempted": 9, "sessions.response_time.p50": 8 }),
    ]);
    expect(store.keys()).toEqual(["dns", "http1"]);
    expect(store.total("sessions.attempted").map((p) => p.v)).toEqual([15, 34]);
    expect(store.of("http1", "sessions.response_time.p50").map((p) => p.v)).toEqual([100, 110]);
    expect(store.latest()?.attempted).toBe(34);
    expect(store.latest()?.failed).toBeUndefined();
  });

  it("ignores other views and a sample with a bad time", () => {
    const store = new MetricsStore();
    store.add([
      { ...sample(1, "http1", { "sessions.attempted": 1 }), view: "port" },
      { ...sample(1, "http1", { "sessions.attempted": 1 }), ts: "yesterday" },
    ]);
    expect(store.size).toBe(0);
    expect(store.latest()).toBeUndefined();
    expect(store.newest()).toBeUndefined();
  });

  it("merges duplicates and late samples into the right place", () => {
    const store = new MetricsStore();
    store.add([sample(3, "a", { m: 3 }), sample(1, "a", { m: 1 })]);
    store.add([sample(2, "a", { m: 2 }), sample(3, "a", { m: 3 })]);
    expect(store.total("m").map((p) => p.v)).toEqual([1, 2, 3]);
    expect(store.newest()).toBe(new Date(T0 + 3000).toISOString());
  });

  it("keeps only the newest ticks", () => {
    const store = new MetricsStore();
    const all: MetricSample[] = [];
    for (let tick = 1; tick <= MAX_TICKS + 5; tick += 1) all.push(sample(tick, "a", { m: tick }));
    store.add(all);
    expect(store.size).toBe(MAX_TICKS);
    expect(store.total("m")[0]?.v).toBe(6);
    expect(store.total("m", 3).map((p) => p.v)).toEqual([
      MAX_TICKS + 3,
      MAX_TICKS + 4,
      MAX_TICKS + 5,
    ]);
  });

  it("tells subscribers once per flush that had news", () => {
    const store = new MetricsStore();
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);
    store.add([sample(1, "a", { m: 1 }), sample(1, "b", { m: 1 })]);
    expect(listener).not.toHaveBeenCalled();
    store.flush();
    store.flush();
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.getSnapshot()).toBe(1);
    unsubscribe();
    store.add([sample(2, "a", { m: 2 })]);
    store.flush();
    expect(listener).toHaveBeenCalledTimes(1);
  });
});
