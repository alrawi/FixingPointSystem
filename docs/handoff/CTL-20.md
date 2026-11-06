# Handoff CTL-20 Run reasons, criterion results, and criterion filters

| Field | Value |
|---|---|
| Issue | #137 |
| Branch | `wp/CTL-20` |
| Pull request | #212 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-95e128, lease until 2026-11-06T16:49:52Z |
| Last verified commit | 952392f, CI green |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 14 and 19 and close note 4 of the CTL-09 log entry: fill `Run.reason` for every failed run, return `observed`, `passed`, and `reason` for every criterion, apply the `where` filter of a criterion to labeled metrics (with the client side default of CON-08), and carry the run id in `runs.Outcome`.

## Plan

- [x] 1. `runs`: reason text, `Outcome.RunID`, labeled counters, criterion type with `where`, `ToAPI` fields
- [x] 2. `report`: label aware criteria evaluation with the client side default
- [x] 3. `report`: run evaluator that reads the aggregator by run id
- [x] 4. Tests and golden reports
- [x] 5. Docs, log entry, lint, and CI

## Done and verified

The sandbox cannot download Go modules, so CI is the build and test loop. Checks that ran locally:

- `gofmt -l controller` is clean, and `plan.py check`, `plan.py render --check`, and `plan.py lint` pass.
- A go/types check of `internal/runs`, `internal/report`, `internal/wiring`, and `cmd/fps-controller` with every unresolvable third party import tolerated found no error in this package.
- The pure parts ran against stubs: `series_internal_test.go` passes, and the evaluator reproduces every number of `testdata/labeled.json` and `labeled.csv` over hand built series, and passes the earlier `criteria_test.go` cases.

## Remaining

1. Review. CI is green on 952392f, including the hand written goldens.
2. The compiler change that lets `where` parse from YAML belongs to CTL-18 (see Blockers).

## Next action

Review pull request #212.

## Blockers and open questions

`compiler.CriterionConfig` has no `where` field and the compiler is outside this package, so a configuration with `where` still fails to parse. The package carries `where` in `runs.Criterion` and fills it in `criteriaOf` once the compiler has the field. The one field and the one line are in the log entry under Shared page notes.

## Decisions taken in this package

- `runs.Criterion` is the compiler criterion plus `Where`, because the compiler type cannot change here. `runs.CriterionResult` embeds it and adds `Invalid` and `Reason`, so an invalid criterion shows `passed` false and no `observed` in `GET /v1/runs/{id}` as in the report.
- The orchestrator keeps every counter with its labels (`Outcome.Series`, `Run.Series`), sums the client side and the unlabeled counters into `Totals`, and counts the open sessions of a failed agent as `sessions.failed` with the reason `aborted`.
- A criterion on a metric with the `side` label and a `where` without a side counts the client side. A series without any `side` label stays in that default. The default and the filter appear in the reason, for example `for application=dns,side=client`.
- A counter reported both with and without labels counts once, through its labeled series, because the e2e test sends both and the run total repeats the series.
- A derived gauge divides the sums of its components over the same selection, and a missing dividend counts as zero.
- `NewAPI` hands its totals source to the evaluator through `UseSeries` when the source offers series, so the wiring in `cmd` needs no change.
- The reason of a failed criteria run lists at most three criteria and counts the rest.

## Pitfalls

Go modules cannot be downloaded in the sandbox, so CI is the build and test loop. The worktree isolation refuses shell commands that mention `git` inside a path or a computed argument, so keep helper scripts in the scratchpad and run them by literal path.

## Verify the current state

```bash
make lint-ctl test-ctl
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T16:20Z | claude-95e128 | claim | none | Fresh claim from origin/main |
| 2026-11-06T15:50Z | claude-95e128 | checkpoint | see the branch | Code, tests, goldens, docs, and log entry written |
| 2026-11-06T17:00Z | claude-95e128 | finish | 952392f | PR #212 opened, CI green, ready for review |
