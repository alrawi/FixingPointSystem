# Handoff CTL-48 The CON-23 property wording for every realism metric

| Field | Value |
|---|---|
| Issue | #500 |
| Branch | `wp/CTL-48` |
| Pull request | #503 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-c4a8e1 |
| Last verified commit | a26f45e (CI green, including ctl `go test`) |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`MapRealism` words `property` of every mapped metric as the examples of `contracts/examples/realism/check-metrics.yaml` do, and the example test compares `property` for every case.

## Plan

- [x] 1. `metricProperty` in `controller/internal/report/realism.go`, used by every rule.
- [x] 2. Tests: `realism_con23_test.go` compares property everywhere, `realism_test.go` follows the new wording.
- [x] 3. Open the PR, merge `origin/main` (CON-28 may change the case counts), wait for CI, request review.

## Done and verified

Steps 1 and 2: `gofmt -l controller/internal/report` is clean. The Go module cannot be downloaded here, so nothing was compiled or run locally.

## Remaining

none. Before merge: if CON-28 landed cases in `check-metrics.yaml`, update the counts in `realism_con23_test.go`.

## Next action

Coordinator review and merge.

## Blockers and open questions

none

## Decisions taken in this package

- The wording depends on the metric and labels only, not on the row that gives target and outcome, so fallbacks of known metrics are worded too.
- A series that lacks a label its wording needs keeps the generic wording.

## Pitfalls

Case counts in `realism_con23_test.go` (10 kinds, 21 cases) change when `check-metrics.yaml` gains cases (CON-28).

## Verify the current state

```bash
gofmt -l controller/internal/report   # empty
# CI: go test ./internal/report
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12 | claude-c4a8e1 | claim and work | | wording, tests, docs |
