# Handoff AGT-02 Agent gRPC service

| Field | Value |
|---|---|
| Issue | #37 |
| Branch | `wp/AGT-02` |
| Pull request | none |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-agt-02-641d, lease until 2026-11-03T17:38:53Z |
| Last verified commit | none |

## Goal

Implement the agent side of the control protocol. The agent dials the controller, calls `Register`, opens the `Session` stream, serves `prepare`, `start`, and `stop` with the state machine of `contracts/README.md`, sends heartbeats, results, and counter snapshots, and dials again with exponential backoff after a lost connection.

## Plan

- [ ] 1. Generated proto types with tonic and protox in a build script
- [ ] 2. A controllable run handle over the standalone run path, fed by a proto `Plan`
- [ ] 3. The session loop with the agent state machine and command results
- [ ] 4. Heartbeats, the heartbeat timeout, and reconnection with backoff
- [ ] 5. A test with an in process fake controller that drives prepare, start, and stop
- [ ] 6. Wiki section, log entry, and finish

## Done and verified

none

## Remaining

Every plan step.

## Next action

Step 1.

## Blockers and open questions

none

## Decisions taken in this package

none

## Pitfalls

none

## Verify the current state

```bash
cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T16:09Z | claude-agt-02-641d | claim | none | Fresh branch from origin/main |
