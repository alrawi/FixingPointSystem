## [2026-11-03] ingest | bug-119 Metrics API and /metrics in the controller binary

We fixed bug #119 and added a Metrics in the binary section at the end of the [Controller](Controller.md) page. The binary `fps-controller` now creates one `metrics.Aggregator`, adds `Aggregator.Observe` to the agent manager observers with `Controller.Observe`, routes the run metrics and the metrics stream to `metrics.NewAPI(agg, ctl.Runs)` with `Controller.SetMetrics`, and serves `metrics.NewPrometheus(agg)` at `GET /metrics` without a token. `TestEndToEnd` sends a snapshot from the fake agent during the run and finds its count in `GET /v1/runs/{id}/metrics` and in `GET /metrics`. The evidence is `make lint-ctl test-ctl` and `docs/handoff/bug-119.md` on the branch `bug/119`.

## Shared page notes

The coordinator may apply three notes.

1. In the Binary wiring section of [Controller](Controller.md), the routing table row for `GET /v1/runs/{id}/metrics` and `:stream` can name `metrics.API` in place of "or 501", and the subsection Hook for run metrics can point to the section Metrics in the binary, because the hook comment is gone.
2. The end to end step list in Binary wiring can gain the metrics step between steps 8 and 9, which reads a fake agent snapshot from the run metrics and from `GET /metrics`.
3. Shared page note 2 of the CTL-08 ingest entry is done, and note 3 of that entry still holds, because the final snapshot of a `StopResult` reaches only the orchestrator.
