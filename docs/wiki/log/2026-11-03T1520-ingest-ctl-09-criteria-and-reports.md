## [2026-11-03] ingest | CTL-09 Criteria evaluation and reports

We added `controller/internal/report` and documented it in the section [Criteria and reports](Controller.md#criteria-and-reports) of the Controller page. The `Evaluator` derives every catalog formula from the merged counters, applies the six operators of the test configuration schema, and gives every criterion the result pass, fail, missing, or invalid with a reason. `Evaluator.RunEvaluator` supplies the `runs.Evaluator` that CTL-07 left open. The type `report.API` wraps `api.Unimplemented` and serves `GET /v1/runs/{id}/report` in JSON, which follows the `RunReport` schema, and in CSV. Golden reports in `controller/internal/report/testdata/` cover a passing run, a run with failing criteria, and a degraded run. The package changes no contract and adds no dependency. The evidence is `docs/handoff/CTL-09.md` on the branch `wp/CTL-09`.

## Shared page notes

The coordinator may apply the notes below.

1. Wiring in `controller/internal/wiring` and `controller/cmd/fps-controller`, which this package may not edit. The evaluator goes into the orchestrator configuration with one line, `Evaluator: ev.RunEvaluator()` in `runs.Config`, after `ev, err := report.NewEvaluator(agg.Catalog())`, where `agg` is the metrics aggregator that the metrics wiring creates. The report handler is `report.NewAPI(ev, orchestrator, agg)`. In `wiring.Server` the method `GetRunReport` of `pending` goes away, and a field `*reportOps` with `reportOps = report.API` joins the embedded handlers, so `TestServerRoutesEveryOperation` then expects six pending operations instead of seven. The Controller page table of the combined server may then name `report.API` for `getRunReport`.
2. [Testing](Testing.md) may add the report goldens in `controller/internal/report/testdata/` to the `test-ctl` row.
3. [Implementation Plan](Implementation-Plan.md) may mark CTL-09 as done after the merge.
4. A follow up for CTL may add the run id to `runs.Outcome`, so that the evaluator reads the aggregator when a run stops and decides histogram criteria such as `sessions.response_time.p99` in the run state as the report already does.
