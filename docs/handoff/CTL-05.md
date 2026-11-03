# Handoff CTL-05 Test compiler

| Field | Value |
|---|---|
| Issue | #43 |
| Branch | `wp/CTL-05` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-ctl-05-1382, lease until 2026-11-03T04:13:46Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We compile a test configuration and an agent inventory into one plan per agent. The compiler normalizes the mix by sessions and splits rates and host pools by core count, and it is deterministic from the seed.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [ ] 2. Add Go types that mirror the canonical JSON mapping of the Plan message
- [ ] 3. Add the compile function with mix normalization and the core based split
- [ ] 4. Add golden tests and determinism tests
- [ ] 5. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

Nothing yet.

## Remaining

Steps 2 to 5.

## Next action

Write the Plan types in `controller/internal/compiler/plan.go`.

## Blockers and open questions

None.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cd controller && GOWORK=off go test -race ./internal/compiler/
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44 | claude-ctl-05-1382 | claim | pending | Claimed with card 6594098855 |
