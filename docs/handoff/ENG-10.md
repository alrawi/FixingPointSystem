# Handoff ENG-10 Worker runtime

| Field | Value |
|---|---|
| Issue | #30 |
| Branch | `wp/ENG-10` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-eng-10-c6ab, lease until 2026-11-03T14:25:52Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Run one single threaded Tokio runtime per worker thread, with optional core pinning. Each worker owns its sessions, timers, and statistics shard. The runtime supports a graceful stop with ramp down and an immediate abort.

## Plan

- [ ] 1. Crate skeleton, worker configuration, and thread spawning with optional pinning
- [ ] 2. Tokio backed clock for fps-sched and the client session loop
- [ ] 3. Server workers over kernel listeners
- [ ] 4. Statistics shards and the one second snapshot to a collector
- [ ] 5. Graceful stop with ramp down and immediate abort, with tests
- [ ] 6. Loopback acceptance test, wiki section, and log entry

## Done and verified

None yet.

## Remaining

Every plan step.

## Next action

Write the crate skeleton under engine/fps-runtime.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cargo test -p fps-runtime
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T12:56Z | claude-eng-10-c6ab | claim | none | fresh claim |
