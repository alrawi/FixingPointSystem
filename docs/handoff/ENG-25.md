# Handoff ENG-25 int_bytes and now in the engine

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #375 |
| Branch | `wp/ENG-25` |
| Pull request | #383 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-65883d, lease until 2026-11-10T14:16:38Z |
| Last verified commit | a6a6e1d |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement the core template functions `int_bytes(value, width, endianness)` and `now(format)` of scenario contract v0.6 (CON-19) in `engine/fps-flow` and `engine/fps-interp`, with a clock seam that tests pin, and add parity fixtures to `verdicts.yaml`. Unit tests cover every width, both byte orders, the error cases, and the three `now` formats under a pinned clock.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Clock seam, `int_bytes`, `now` in the fps-flow template compiler and renderer, with unit tests
- [x] 2. Parity fixtures in `verdicts.yaml` and the fixture scenarios
- [x] 3. fps-interp: clock seam in `Session` and `ScenarioInstance`, variables across `after` test, interp tests
- [x] 4. Checks, log entry, Engine page, PR

## Done and verified

- Step 1: `engine/fps-flow/src/clock.rs` (Clock, SystemClock, PinnedClock, http_date), `int_bytes` and `now` in the template parser, checker, folder, renderer, and dump. `cargo test -p fps-flow --lib` passes (new tests in `template/tests_core.rs`, including 2 proptests).
- Step 3: `Session::set_clock`, `ScenarioInstance::set_clock` (kept across a rerun), `tests/core_functions.rs` (6 tests), and three variable tests in `tests/instances.rs`. `cargo test -p fps-flow -p fps-interp` passes, `cargo clippy -p fps-flow -p fps-interp --all-targets -- -D warnings` is clean, and `cargo check -p fps-runtime --all-targets` compiles.
- Step 4: `plan.py check`, `render --check`, `lint`, and `tools/contracts/validate.py` pass; the log entry and the Engine page are written.
- Step 2: 14 fixture scenarios under `tests/fixtures/modules/fx/scenarios/` (core_int_bytes_ok, core_now_ok, 12 rejects) and their verdicts. `cargo test -p fps-flow --test fixtures` and `python3 -m pytest tests/test_fixtures.py` pass (64 Python cases).

## Remaining

None.

## Next action

Review PR #383. CI is green on a6a6e1d.

## Blockers and open questions

none

## Decisions taken in this package

- `RenderCtx` gains a required field `clock: &dyn Clock`. It is built only in fps-flow and fps-interp, so no crate outside the package paths needs a change. `SessionCtx` and `InstanceConfig` keep their fields (fps-runtime, fps-agent, and the kernel backend build them), so the clock enters through `Session::set_clock` and `ScenarioInstance::set_clock`, with the wall clock as the default.
- `int_bytes` with known valid arguments folds into literal bytes. With an invalid folded variable it stays and fails when rendered, so a bad variable is a `template_error` and not a load error.
- A literal argument that breaks a rule is a compile error of the template (`LoadError::Template`), which matches the validator. `now` never folds.

## Pitfalls

- `ScenarioInstance::run` rebuilds the instance with `reset()`, which first dropped the clock. It now carries the clock over.
- The sandbox worktree guard refuses heredocs next to `git`; write scripts with the editor tools.

## Verify the current state

```bash
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=1
cargo test -p fps-flow -p fps-interp   # all pass
(cd engine/fps-flow && python3 -m pytest -q tests/test_fixtures.py)   # 64 passed
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T13:47Z | claude-65883d | claim | none | claimed #375 |
| 2026-11-10T14:00Z | claude-65883d | finish | a6a6e1d | PR #383 open, CI green (the first push failed the deprecated fetch_update lint of the newer CI toolchain; fixed with fetch_add) |
