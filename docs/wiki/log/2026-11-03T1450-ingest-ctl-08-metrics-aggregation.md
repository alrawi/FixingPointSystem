## [2026-11-03] ingest | CTL-08 Metrics aggregation and streaming

We added the metrics package `controller/internal/metrics` and a Metrics aggregation section at the end of the [Controller](Controller.md) page. The aggregator keeps the newest series of every agent and worker of a run, merges counters, gauges, and HDR histograms, recomputes derived ratios, and records the six views once per second. The type `metrics.API` embeds `api.Unimplemented` and serves `GET /v1/runs/{id}/metrics` with views and windows and `GET /v1/runs/{id}/metrics:stream` as Server Sent Events, and `metrics.Prometheus` serves `/metrics`. Counters equal the sums of fake agent snapshots, and the histogram merge matches a reference merge that `engine/fps-stats` wrote. The evidence is `make lint-ctl test-ctl` and `docs/handoff/CTL-08.md` on the branch `wp/CTL-08`.

## Shared page notes

The coordinator may apply five notes.

1. The Layout table of [Controller](Controller.md) can gain the row `internal/metrics/` with the purpose "Snapshot aggregation, the metrics views and stream, and Prometheus exposition".
2. CTL-17 can wire the package. It creates one `metrics.New(metrics.Config{Clock: clock, Logger: logger})`, calls `Aggregator.Observe` from `agents.Config.Observer` next to `Orchestrator.Observe`, embeds `metrics.NewAPI(agg, orchestrator)` with the other operation sets, and mounts `metrics.NewPrometheus(agg)` at `GET /metrics`.
3. The final snapshot that a `StopResult` carries reaches only the orchestrator. Once a hook exists in `internal/runs`, it can pass that snapshot to `Aggregator.Add`, which today misses at most the last counter interval of a stopped agent.
4. The OpenAPI document does not list `/metrics`, and the generated `MetricSample` holds float32 values. A contract change could add the endpoint and give `values` the format double, after which the package can use the generated type.
5. CTL-09 can read `Aggregator.Totals`, which aggregates every metric over every label with histogram statistics under `<metric>.<stat>`, as test criteria name them. [Home](Home.md) can mark CTL-08 as ready for review.
