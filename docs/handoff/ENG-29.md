# Handoff ENG-29 The raw action in the engine

| Field | Value |
|---|---|
| Issue | #523 |
| Branch | `wp/ENG-29` |
| Pull request | none yet |
| Status | in progress |
| Progress | 85% (3 of 4 plan steps) |
| Current holder | claude-7e29a1, lease until 2026-11-12T21:02:13Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Scenario contract v0.6 defines the action `raw` (hex bytes written without templating) and reserves it for strike and fuzzer modules. The contract check of the engine admits it only there, and the interpreter refuses a runnable program with `raw` as an unsupported action. The interpreter must write the bytes of a raw action on the session, a raw action in any other module type must fail to load with a named error, and the text for `docs/wiki/Module-Development.md` goes in the Shared page notes of the log entry.

## Plan

- [x] 1. Compile `raw` to a literal send in `engine/fps-flow/src/program.rs`, and update the load tests and the fixture test that tolerate `Unsupported`.
- [x] 2. Test in `engine/fps-runtime/tests/` that a session of a strike module sends the exact bytes of a raw action over TCP (UDP is covered at the compiled program level).
- [x] 3. Test that a raw action in an app module fails to load with `ReservedAction`.
- [ ] 4. Docs: Engine page, log entry with the Shared page notes, then checks, PR, CI, review.

## Done and verified

- `cargo test -p fps-flow`: all pass, including the new unit tests of `program.rs` and the two load tests of `tests/fixtures.rs`.
- `cargo test -p fps-runtime --test raw`: 3 passed (exact bytes of 5 strike sessions, app module refused).
- `cargo clippy -p fps-flow -p fps-runtime --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `plan.py check`, `render --check`, `lint`: clean.

## Remaining

Step 4: PR, CI, review. The full `cargo test -p fps-runtime` runs in CI only.

## Next action

Open the PR and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- UDP is covered only at the compiled program level; the runtime test uses TCP because a flow without a server side needs the instance runner for UDP.
- `raw` compiles to `Op::Send` of `Template::literal(bytes)`, so no new op exists and `fps-interp` (outside this package's paths) needs no change. A literal segment is never parsed as a template, so `{{` in the bytes stays literal.

## Pitfalls

- `close: fin` drains until the peer closes, so a test listener must close its side or the client session fails with `expect_timeout`.
- The worktree guard refuses `source` and `export` in a command; use `VAR=value command`.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-ENG-29 CARGO_BUILD_JOBS=1 cargo test -p fps-flow
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T19:35Z | claude-7e29a1 | claim | none | Claimed #523, card 6833837021 |
