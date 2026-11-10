# Handoff ENG-23 Multi flow scenario instances in the runtime

| Field | Value |
|---|---|
| Issue | #349 |
| Branch | `wp/ENG-23` |
| Pull request | #354 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-e23a7c, lease until 2026-11-10T02:37:54Z |
| Last verified commit | caf65d1 |

## Goal

`fps-interp` has `ScenarioInstance` and `spawn`, but `fps-runtime` has no worker that drives an instance, so the agent refuses scenarios with more than one flow and the FTP module cannot run (AGT-13 finding). Add a worker mode that runs every flow of an instance through a `FlowEnv` of the kernel backend, with session ids, metric series, capture, and extensions for every flow, and an opener that binds a spawned data port on accept and tolerates a connect that arrives first.

## Plan

- [x] 1. `instance.rs` with the FlowEnv, the openers, and the worker hook; single flow scenarios keep today's path
- [x] 2. Config check for instance scenarios (entry flow, no UDP flow)
- [x] 3. Loopback test of `modules/ftp` `mixed_session` with one data connection per transfer
- [x] 4. Tests for the openers (connect before listener, port in use), metrics, extensions, capture
- [x] 5. `docs/wiki/Engine.md`, log entry, checks, PR

## Done and verified

`cargo test -p fps-runtime` passes: 8 test binaries, including `tests/instances.rs` (6 tests, `mixed_session` with 32 sessions and 32 captured connections) and 3 instance opener unit tests. `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Nothing.

## Next action

Review PR #354.
