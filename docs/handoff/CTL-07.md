# Handoff CTL-07 Run orchestrator

| Field | Value |
|---|---|
| Issue | #45 |
| Branch | `wp/CTL-07` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-ctl-07-8293, lease until 2026-11-03T14:25:52Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We drive the run state machine from pending to a final state and contain agent faults. The controller serves POST /v1/runs, GET /v1/runs/{id}, and POST /v1/runs/{id}:stop, and it marks a run degraded when an agent fails during the run.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [ ] 2. Add the state machine with its transition table and table driven tests
- [ ] 3. Add the orchestrator over store, compiler, and agent manager interfaces
- [ ] 4. Serve the three run operations and test them with fake agents
- [ ] 5. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

Nothing yet.

## Remaining

Steps 2 to 5.

## Next action

Write the state machine in `controller/internal/runs/state.go`.

## Blockers and open questions

None.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cd controller && GOWORK=off go test -race ./internal/runs/...
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T12:56Z | claude-ctl-07-8293 | claim | none | Claimed from status:ready |
