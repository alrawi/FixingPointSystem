## [2026-11-09] ingest | CTL-35 List runs

`GET /v1/runs` (`listRuns`) no longer answers 501. `runs.API` serves it from the new `Orchestrator.List`, which returns the runs newest first with the `state` filter, `page_size` (1 to 500, default 50), and an opaque `page_token`. The order follows a creation sequence that the orchestrator assigns under its lock, and the token names the last sequence of the page, so a run that starts between two requests never shifts or repeats an item. A bad `state`, `page_size`, or token is a 400 problem. The wiring drops the pending stub, and `pendingOps` of the wiring test loses `listRuns`. The contract tests validate every response against the OpenAPI document, and the CLI end to end test now expects exit 0 from `fps run list` and the started run in its output. We described the operation in [Controller](Controller.md) and [CLI](CLI.md).

The package changed `cli/e2e/e2e_test.go`, which is outside its catalog paths, because its acceptance names that test.

### Shared page notes

- Testing: `TestServerRoutesEveryOperation` now expects two pending operations, `getRunMetrics` and `streamRunMetrics`, if a count is stated anywhere.
- Contracts: none. `listRuns` is implemented exactly as the OpenAPI document defines it.
- Implementation Plan: CTL-35 is done once merged.
