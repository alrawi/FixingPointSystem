# Handoff ENG-27 Dropped counts of a capture

| Field | Value |
|---|---|
| Issue | #424 |
| Branch | `wp/ENG-27` |
| Pull request | #432 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-e27a41, lease until 2026-11-11T00:05:31Z |
| Last verified commit | 02f41fa |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`StopCaptureResult.dropped` is documented as packets that the capture missed, but it also counts every session still open when the capture stops (AGT-18 found this). Count queue overflow drops and sessions open at the stop apart inside `engine/fps-capture/`, and document which is which. The agent protocol field `dropped` keeps its meaning.

## Plan

- [x] 1. Split the counter in `engine/fps-capture/src/lib.rs` and document it.
- [x] 2. Tests: an overflowing queue and a stop during a run, with literal counts.
- [x] 3. Checks, log entry, Engine page update.
- [x] 4. Pull request and CI.

## Done and verified

- `cargo test -p fps-capture` passes (22 unit, 15 component, 1 doc test); `cargo clippy -p fps-capture --all-targets -D warnings` and `cargo fmt --check` are clean; `plan.py check`, `render --check`, `lint` pass.

## Remaining

Nothing.

## Next action

Coordinator review and merge.

## Blockers and open questions

none

## Decisions taken in this package

- `sessions_dropped_queue` keeps its value (overflow plus cut at the stop), because `engine/fps-agent` adds it to `sessions_dropped_oversize` for `StopCaptureResult.dropped`, and the agent is outside this package. Two new fields, `sessions_dropped_overflow` and `sessions_cut_at_stop`, split it.

## Pitfalls

none

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=$PWD/target-ENG-27 cargo test -p fps-capture
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T22:36Z | claude-e27a41 | claim | none | claimed #424 |
| 2026-11-10T23:20Z | claude-e27a41 | ready for review | 02f41fa | PR #432, CI green |
