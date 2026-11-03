## [2026-11-03] ingest | bug-124 Criteria evaluator and run reports in the controller binary

We fixed bug #124 and added a section [Reports in the binary](Controller.md#reports-in-the-binary) at the end of the [Controller](Controller.md) page. `wiring.New` now creates the CTL-09 evaluator with `report.NewEvaluator(nil)`, exposes it as `Controller.Evaluator`, and passes `Evaluator.RunEvaluator()` to the run orchestrator. `wiring.Server` forwards `getRunReport` to a `ReportHandler`, which defaults to `report.NewAPI(ev, ctl.Runs, nil)`, and the new hook `Controller.SetReport` lets `mountMetrics` in the binary replace it with `report.NewAPI(ctl.Evaluator, ctl.Runs, agg)`. `TestEndToEnd` fetches the JSON and CSV report after the run stops and checks the results of two criteria, one of which only the CTL-09 evaluator can decide. The evidence is `make lint-ctl test-ctl` and `docs/handoff/bug-124.md` on the branch `bug/124`. The fix changes no contract and adds no dependency.

## Shared page notes

The coordinator may apply the notes below.

1. In the Binary wiring section of [Controller](Controller.md), the routing table row for `listRuns`, captures, and `getRunReport` can drop `getRunReport` and gain a row that names the `ReportHandler` of `Controller.SetReport`, which defaults to `report.API`. The sentence on `TestServerRoutesEveryOperation` can say six pending operations in place of seven.
2. Shared page note 1 of the CTL-09 ingest entry is done.
3. Note 4 of the CTL-09 ingest entry still holds. The run state verdict comes from the unlabeled totals of the orchestrator, so a histogram criterion such as `sessions.response_time.p99` reads missing in `GET /v1/runs/{id}` while the report decides it from the aggregator.
