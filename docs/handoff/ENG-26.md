# Handoff ENG-26 Datagram flows in multi flow instances and data ports without repeats

| Field | Value |
|---|---|
| Issue | #422 |
| Branch | `wp/ENG-26` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-d50d53, lease until 2026-11-11T00:05:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Give the instance runner of `engine/fps-runtime` a datagram opener (bound server socket per port shared by its sessions, a socket per media port bound for the flow, a connected client socket with its own source port per flow), and stop two concurrent instances that draw the same `rand_int` data port from colliding on one listener (AGT-18). Do not edit `engine/fps-agent/` (AGT-20) or `modules/`.

## Plan

- [ ] 1. Read the runtime, fixture_peer.rs, and the MOD-14 and AGT-18 log entries
- [ ] 2. Datagram opener in InstanceEnv
- [ ] 3. Data port collision fix
- [ ] 4. Runtime tests: SIP call with RTP through the instance runner, two instances on one drawn port
- [ ] 5. Checks, log entry, Engine page
- [ ] 6. PR and CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read engine/fps-runtime/src/instance.rs and opener.rs.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=1 cargo test -p fps-runtime
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T22:36Z | claude-d50d53 | claim | none | Claimed #422 with card 6781155090 |
