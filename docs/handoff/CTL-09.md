# Handoff CTL-09 Criteria evaluation and reports

| Field | Value |
|---|---|
| Issue | #47 |
| Branch | `wp/CTL-09` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-ctl-09-ca2d, lease until 2026-11-03T16:39:53Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Evaluate the pass and fail criteria of a run against the merged run metrics and serve the result as a report at `GET /v1/runs/{id}/report` in JSON and CSV. Golden reports cover a passing run, a failing criterion, and a degraded run.

## Plan

- [ ] 1. Read contracts, the metrics aggregator of CTL-08, and the runs evaluator of CTL-07
- [ ] 2. Criteria evaluation with every operator and the catalog ratio formulas
- [ ] 3. The report model and the JSON and CSV encoders
- [ ] 4. The report handler, the runs evaluator adapter, and golden tests
- [ ] 5. Docs, lint, tests, and finish

## Done and verified

none

## Remaining

Every plan step.

## Next action

Read the contracts and the metrics aggregator.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cd controller && GOWORK=off go test ./internal/report/...
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T15:10Z | claude-ctl-09-ca2d | claim | none | Fresh claim from origin/main |
