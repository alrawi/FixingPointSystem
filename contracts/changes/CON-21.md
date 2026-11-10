# CON-21 Scenario totals, criterion filters, and realism results in the REST report

Contract. Controller OpenAPI document `contracts/openapi/fps-api-v1.yaml`. The change takes findings of UI-05 (`docs/wiki/log/2026-11-10T0800-ingest-ui-05-reports-and-run-comparison.md`) and RLM-07 (`realism/README.md`, section Mix accuracy). The owner approved it on 2026-11-10. Every change is additive.

Versions.

- OpenAPI moves from 0.8.0 to 0.9.0. The change adds properties and schemas that a reader can see in the document, so it takes a minor number, as CON-12, CON-14, CON-15, and CON-20 did. `api.APIVersion` follows, because `capabilities_test.go` compares it with `info.version`. `openapi/fps-metrics.yaml` stays at 0.3.0.
- The agent protocol (0.2.3), the scenario contract (v0.6), the test configuration (v0.3), the module manifest schema (v0.4), and the metric catalog (0.4.0) stay as they are.

`RunReport.scenarios` (UI-05 and RLM-07 finding). New optional array of the new schema `ScenarioTotals`, ordered by application and then by scenario name. One entry holds the counts of one scenario of one application over the whole run.

| Property | Type | Metric (`x-fps-metric`) |
|---|---|---|
| `application` | string, required | The `application` label, the module name |
| `scenario` | string, required | The `scenario` label, the name within the module |
| `sessions_attempted` | integer, at least 0, required | `sessions.attempted` |
| `sessions_succeeded` | integer, at least 0, required | `sessions.succeeded` |
| `sessions_failed` | integer, at least 0, required | `sessions.failed`, summed over the `reason` label |
| `bytes_tx` | integer, at least 0, required | `bytes.tx` |
| `bytes_rx` | integer, at least 0, required | `bytes.rx` |

- The names follow `RunSummary` (`sessions_attempted`, `sessions_failed`, `bytes_tx`, and so on), which CTL-38 reads as the existing naming of the report structs. A scenario table of CTL-38 with `sessions` and `bytes` maps onto it as `sessions_attempted` and `bytes_tx` plus `bytes_rx`.
- The entries are the client side series of the metric view `scenario` (the key of a sample there is `<application>/<scenario>`). Each session and each byte counts once, as in the totals of the report, and the sum of a property over all entries equals the property of `run.summary` with the same name. The totals of one application are the sum of its entries, so the contract needs no second array.
- A controller of 0.9.0 or later always sends `scenarios`, empty when the run has recorded no scenario counter. A controller older than 0.9.0 does not send it, which is why it is not in `required`. A count is 0 when the run recorded no event for it, and a scenario appears once the run has recorded any counter for it.
- RLM-07 reads `{"scenarios": [{"scenario": "http1/web_get", "sessions": 600, "bytes": 123456}]}`. The `RunReport` entry carries `application` and `scenario` apart, as the catalog labels do, so the realism runner joins them with `/`, reads `sessions` from `sessions_attempted`, and reads `bytes` as `bytes_tx` plus `bytes_rx`. Both are client side values, which are the two directions of the same sessions, so each byte counts once, as the README of the runner says for the metric series. This is a small RLM change, not a contract change.

`CriterionResult.where` (UI-05 finding). New optional object (string values, at least one entry). It is the label filter of the criterion, copied as the test configuration states it, for example `{application: dns, side: server}`. It is absent for a criterion without a filter. The controller does not add a `side` entry that the configuration left out, and the rule of CON-15 stays: a criterion without `side` counts the client side only for a metric that has the side label. Two results with the same metric, op, and value now differ by this field, so a reader pairs the criteria of two runs by `(metric, op, value, where)` and no longer by order.

`RunReport.realism` (UI-05 finding). The shape does not change, and the schema is named `RealismResult` in place of the anonymous item type, which makes no difference in a document and gives the SDK and the UI one type name. Two optional properties are new: `metric` (a name from the catalog) and `labels` (the labels of the series). A reader pairs results of two runs by `metric` and `labels` when they exist and by `property` otherwise. The text of the property now says where the entries come from.

- The controller does not measure realism. The realism runner scores the capture of the run (`python3 -m realism score`) and writes a realism report (`realism/schema/realism-report.schema.json`). Each metric of a check of that report, together with the verdict of the check, is one entry: `observed` is the value, `target` the rule it had to meet (for conformance the threshold, as in `>= 0.99`), and `passed` the outcome.
- The same values are the `realism.` gauges of the metric view `realism` and the `mix.` gauges with `source: classifier` (CON-20, RLM-13), when the controller holds them as series.
- Until the controller has such a report for the run, `realism` is the empty array, which is the answer today for every run. The controller has no way to receive the report yet, and this change adds no operation for it. How the report reaches the controller (the runner could post it, or the controller could read it from the capture directory) needs its own package, and an owner decision when it adds an operation. The contract text says that the way of delivery does not change the schema, so such a package needs no further change to the report.

Compatibility. A client that ignores `scenarios`, `where`, `metric`, and `labels` sees no difference. No property becomes required in a request or leaves a response, no type changes, and every enum and status code stays. `oasdiff breaking` reports nothing, because the change adds optional response properties, two schemas, and descriptions. The existing mock and fixtures stay valid, since a document without the new properties satisfies the schema. `python3 tools/contracts/validate.py` and `python3 -m pytest -q tools/contracts` pass.

Consumers.

| Consumer | Package | Uses |
|---|---|---|
| Controller | CTL-39 | Fills `scenarios` from the table that CTL-38 adds to the report, copies the filter into `where` for each criterion, and fills `realism` when it holds results. A report API test with literal scenario totals and a realism result is its acceptance |
| Controller | CTL-38 (parallel) | The scenarios table of its report model. The field names above are the ones to use, so that CTL-39 only maps them. CTL-38 does not need to wait for this change |
| Web UI | The report and comparison views of UI-05, and the next UI package that touches reports | Shows the scenario table and the per application totals, pairs criteria by `where`, and pairs realism results by `metric` and `labels`. The mock must serve the new properties and keep validating against the document |
| Python SDK | `sdk/python` | `RunReport.scenarios`, `CriterionResult.where`, and the `RealismResult` type |
| CLI | `cli/internal/fpscli` | `fps report` can print the scenario table. Its own `RunReport` struct ignores the new properties until then |
| Realism runner | RLM-07 follow up | Reads the `scenarios` table of the report as a run report form in place of the metric series |

Follow up outside the catalog paths (the coordinator applies them on the branch, and this package does not edit them).

- Controller: `controller/internal/api/capabilities.go` `APIVersion` moves from `0.8.0` to `0.9.0`, which `capabilities_test.go` (`doc.Info.Version != api.APIVersion`) checks. `controller/internal/api/api.gen.go` is regenerated with `go generate ./internal/api`. It gains `ScenarioTotals`, `RealismResult`, `RunReport.Scenarios`, and `CriterionResult.Where`, and `RunReport.Realism` changes type from an anonymous struct slice to `[]RealismResult`. `controller/internal/report/report.go` then no longer compiles at `Report.API`, which builds `out.Realism` with the anonymous struct. The smallest fix is `out.Realism = []api.RealismResult{}` and `out.Scenarios = []api.ScenarioTotals{}`, and CTL-39 fills both.
- Python SDK: `python3 sdk/python/scripts/generate_client.py` rewrites `sdk/python/fixingpoint/_generated.py`. The only change besides the version and the digest is `where` on `CriterionResult`, the new `RealismResult` and `ScenarioTotals`, and `scenarios` on `RunReport`. The type `RunReportRealismItem` disappears, and nothing imports it.
- UI: `ui/src/api/schema.d.ts` is regenerated with `npm run gen:api` in `ui`. `ui/src/api/client.ts` defines `RealismResult` as `RunReport["realism"][number]`, which keeps working. The pairing of criteria and realism results in `ui/src/reports/compare.ts` and the scenario table are a UI package.
- `tools/contracts/test_contracts.py` pins no OpenAPI version, so nothing changes there.
- Wiki: `docs/wiki/Controller.md` (REST report section) and `docs/wiki/Web-UI.md` describe the report and mention the empty realism array, which the owners of those pages update with CTL-39 and the UI package.
