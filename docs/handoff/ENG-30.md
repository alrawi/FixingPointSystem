# Handoff ENG-30 Count a scenario session once, with the bytes of its spawned flows

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #535 |
| Branch | `wp/ENG-30` |
| Pull request | #542 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-d3e952, lease until 2026-11-13T06:39:19Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement CON-31 in `engine/fps-runtime`: a session is one run of the entry flow of a scenario instance, every other flow adds its bytes and its failure to it, and `sessions.concurrent` and `sessions.duration` span the instance. Update the tests that count sessions of multi flow scenarios, and Engine.md.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Engine: instance.rs and worker.rs record one session per instance
- [x] 2. fps-runtime tests (instances.rs, datagrams.rs, tls_sessions.rs)
- [x] 3. fps-agent tests (first_party.rs, controller_e2e.sh)
- [x] 4. Engine.md and the log entry with the Shared page notes
- [x] 5. Full checks, PR, CI, review

## Done and verified

- Steps 1 to 4: `cargo test --workspace` (1487 passed, 0 failed), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `plan.py check`, `plan.py render --check`, `plan.py lint` all pass locally. `controller_e2e.sh` passes `bash -n`; it needs namespaces and root, so only CI runs it.

## Remaining

PR, CI, review step. The coordinator applies the Module-Development text of the log entry before merge.

## Next action

Open the PR, wait for CI, then run `fps_swarm.py review 535 --pr <PR>`.

## Blockers and open questions

none

## Decisions taken in this package

See the log entry `docs/wiki/log/2026-11-13T0500-ingest-eng-30-count-a-scenario-session-once.md`.

## Pitfalls

The disk of the sandbox filled at 19 GB of debug builds (ld died with a bus error). Build with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-ENG-30 cargo test -p fps-runtime   # all pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-13T05:10Z | claude-d3e952 | claim | - | claimed #535, branch from origin/main 61ff082 |
| 2026-11-13T06:05Z | claude-d3e952 | checkpoint | - | engine, tests, Engine.md, log entry done; workspace tests, clippy, fmt pass; opening PR |
| 2026-11-13T06:30Z | claude-d3e952 | review | b49213e | PR #542, CI green |
