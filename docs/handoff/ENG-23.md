# Handoff ENG-23 Multi flow scenario instances in the runtime

| Field | Value |
|---|---|
| Issue | #349 |
| Branch | `wp/ENG-23` |
| Pull request | none yet |
| Status | in progress |
| Progress | 10% (0 of 5 plan steps) |
| Current holder | claude-e23a7c, lease until 2026-11-10T02:37:54Z |
| Last verified commit | none |

## Goal

`fps-interp` has `ScenarioInstance` and `spawn`, but `fps-runtime` has no worker that drives an instance, so the agent refuses scenarios with more than one flow and the FTP module cannot run (AGT-13 finding). Add a worker mode that runs every flow of an instance through a `FlowEnv` of the kernel backend, with session ids, metric series, capture, and extensions for every flow, and an opener that binds a spawned data port on accept and tolerates a connect that arrives first.

## Plan

- [ ] 1. `instance.rs` with the FlowEnv, the openers, and the worker hook; single flow scenarios keep today's path
- [ ] 2. Config check for instance scenarios (entry flow, no UDP flow)
- [ ] 3. Loopback test of `modules/ftp` `mixed_session` with one data connection per transfer
- [ ] 4. Tests for the openers (connect before listener, port in use), metrics, extensions, capture
- [ ] 5. `docs/wiki/Engine.md`, log entry, checks, PR

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Build `fps-runtime` and fix compile errors in `engine/fps-runtime/src/instance.rs`.
