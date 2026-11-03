// SPDX-License-Identifier: GPL-3.0-only

// Package metrics merges the counter snapshots of the agents of a run into
// run metrics and serves them. It implements CTL-08.
//
// Agents send cumulative CounterSnapshot messages on their Session stream,
// one per agent or one per worker when the plan asks for per worker
// snapshots. The Aggregator keeps the newest value of every series of every
// agent and worker, keyed by sequence, so duplicates after a reconnection and
// late final snapshots change nothing. A merge sums counters and additive
// gauges, averages ratio gauges, merges HDR histograms bucket by bucket, and
// recomputes the derived gauges of the catalog, such as sessions.failed_ratio,
// from the merged counters.
//
// The package serves three surfaces.
//
//   - API answers GET /v1/runs/{id}/metrics with the samples of one view over
//     a time window, from one point per second that the aggregator records.
//   - API answers GET /v1/runs/{id}/metrics:stream with Server Sent Events.
//   - Prometheus answers GET /metrics in the text exposition format.
//
// The binary wires the package with three calls.
//
//	agg, _ := metrics.New(metrics.Config{Clock: clock, Logger: logger})
//	// agents.Config.Observer calls agg.Observe(ev) next to the orchestrator.
//	ops := metrics.NewAPI(agg, orchestrator) // embeds api.Unimplemented
//	mux.Handle("GET /metrics", metrics.NewPrometheus(agg))
//
// Metric names, kinds, units, levels, and labels come from an embedded copy
// of contracts/metrics/catalog.yaml, and histogram values decode from the
// HdrHistogram V2 encoding that engine/fps-stats writes.
package metrics
