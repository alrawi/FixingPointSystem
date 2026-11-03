# Handoff CTL-09 Criteria evaluation and reports

| Field | Value |
|---|---|
| Issue | #47 |
| Branch | `wp/CTL-09` |
| Pull request | #122 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-ctl-09-ca2d, lease until 2026-11-03T16:50:41Z |
| Last verified commit | see the session log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Evaluate the pass and fail criteria of a run against the merged run metrics and serve the result as a report at `GET /v1/runs/{id}/report` in JSON and CSV. Golden reports cover a passing run, a failing criterion, and a degraded run.

## Plan

- [x] 1. Read contracts, the metrics aggregator of CTL-08, and the runs evaluator of CTL-07
- [x] 2. Criteria evaluation with every operator and the catalog ratio formulas
- [x] 3. The report model and the JSON and CSV encoders
- [x] 4. The report handler, the runs evaluator adapter, and golden tests
- [x] 5. Docs, lint, tests, and finish

## Done and verified

Every step is verified by `make lint-ctl test-ctl` with `GOWORK=off`, which passed with 0 lint issues and every controller package green. The package coverage is 94.5%. `plan.py check`, `render --check`, and `lint` pass.

| Acceptance | Test |
|---|---|
| Golden report of a passing run | `TestGoldenReports/pass` with `testdata/pass.json` and `testdata/pass.csv` |
| Golden report of a failing criterion | `TestGoldenReports/fail` with `testdata/fail.json` and `testdata/fail.csv` |
| Golden report of a degraded run | `TestGoldenReports/degraded` with `testdata/degraded.json` and `testdata/degraded.csv` |

Every golden response is validated against the OpenAPI contract through the generated router.

## Remaining

1. Wiring in `internal/wiring` and `cmd/fps-controller` is outside the package paths. The log entry `docs/wiki/log/2026-11-03T1520-ingest-ctl-09-criteria-and-reports.md` describes it under Shared page notes.
2. `runs.Outcome` has no run id, so `RunEvaluator` cannot read histogram statistics from the aggregator at stop time, and such criteria are missing in the run state while the report decides them. A follow up can add the run id to `runs.Outcome`.
3. The `realism` list of the JSON report stays empty until the realism pipeline delivers results, and the HTML format answers 501.
4. The contract verdict items have no field for the result and reason of a criterion, so the JSON report shows missing criteria without `observed` and `passed`, and the CSV report carries the result and reason.

## Next action

A reviewer checks pull request #122 against the Definition of Done.

## Blockers and open questions

none

## Decisions taken in this package

- A criterion on a catalog metric without a value is missing and does not decide the verdict, as in CTL-07, because gauges such as `mix.max_abs_error_pp` have no producer yet. A criterion that names no catalog metric or operator is invalid and fails the run.
- Derived gauges are computed again from the merged counters, and a ratio over a zero divisor is undefined, matching the aggregator of CTL-08. The CTL-07 default reported 0 in that case.
- In a report, counters of the orchestrator take precedence over the aggregator, because the orchestrator counts the open sessions of a failed agent as failed. Gauges and histogram statistics come from the aggregator, which averages ratio gauges where the orchestrator sums them.
- The JSON report keeps to the generated `RunReport` type, so the package adds no contract change.
- The CSV report is one table so spreadsheets and scripts read it without sections, writes numbers in plain decimal notation, and guards text cells against formula injection.

## Pitfalls

- `agentpb.CounterValue.Labels` is a map, not a list of label messages.
- The contract validator rejects a request with an unknown `format`, so the 400 path is tested by calling the handler directly.

## Verify the current state

```bash
cd controller && GOWORK=off go test -race ./internal/report/...   # ok
make lint-ctl test-ctl                                             # 0 issues, all ok
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T15:10Z | claude-ctl-09-ca2d | claim | none | Fresh claim from origin/main |
| 2026-11-03T15:22Z | claude-ctl-09-ca2d | finish | see git log | Lint and tests green, ready for review |
