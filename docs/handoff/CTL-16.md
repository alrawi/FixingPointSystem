# Handoff CTL-16 Multi agent runs

| Field | Value |
|---|---|
| Issue | #567 |
| Branch | `wp/CTL-16` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-f9c390, lease until 2026-11-17T00:23:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Split plans across several agents, check clock synchronization before the start, and aggregate the results of the agents. Acceptance: a run with three fake agents aggregates counters exactly and refuses to start with a clock offset above 10 ms.

## Plan

- [ ] 1. Package skeleton, errors, and the clock offset check (`clock.go`)
- [ ] 2. Exact counter aggregation across agents (`aggregate.go`)
- [ ] 3. Plan splitting wrapper (`split.go`)
- [ ] 4. The multi agent run: check, reserve, prepare, start at a common instant, stop, aggregate (`run.go`)
- [ ] 5. Tests with three fake agents, wiki page and log entry, PR

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Write `controller/internal/multiagent/clock.go`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

Go modules cannot be downloaded in this sandbox, so Go code is checked with `gofmt -l` locally and built by the CI of the pull request.

## Verify the current state

```bash
gofmt -l controller/internal/multiagent/
```

Expected result: no output.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16T22:53Z | claude-f9c390 | claim | none | Claimed #567 with card 6931811598 |
