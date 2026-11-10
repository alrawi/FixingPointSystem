# Handoff AGT-18 The run_control capture flake

| Field | Value |
|---|---|
| Issue | #389 |
| Branch | `wp/AGT-18` |
| Pull request | #418 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-048417, lease until 2026-11-10T19:14:44Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Find and fix the cause of the dropped packet in `a_fake_controller_changes_rates_and_reads_a_capture` (`engine/fps-agent/tests/run_control.rs`) and the cause of the lost ftp sessions when the tests of `tests/first_party.rs` run in parallel, then remove the mutex or justify it. Both tests pass 50 times in a row in CI, and the capture test still fails when a packet is really dropped.

## Plan

- [x] 1. Reproduce and fix the capture flake
- [x] 2. Reproduce the ftp session loss and fix it, remove the mutex
- [x] 3. Repeat tests for the loop proof, checks, wiki log entry
- [x] 4. Open the PR, run the repeat tests in CI, CI green, review

## Done and verified

1. Capture flake. A session that is open when the capture stops is counted as dropped: it ends after the writer thread has gone, and `Shared::drop` in `engine/fps-capture/src/lib.rs` finds the channel disconnected. A temporary diagnostic showed every drop was `Disconnected`, never `Full`. Three busy loops beside the test made 3 of 4 runs fail with `dropped: 1` or `2`. The test now pauses both entries and waits for `sessions.concurrent` to be zero before it stops the capture (`quiesce`), and 4 of 4 runs under the same load passed. `a_capture_that_loses_sessions_reports_them_as_dropped` shows that a really lost session reaches `StopCaptureResult.dropped`. The ignored `repeat_stopping_a_capture_at_rest` stops a capture at rest 50 times in a row: 50 of 50 under three busy loops, 132 s.
2. ftp sessions. The 2 lost sessions are one instance: two instances drew the same data port and one connection was reset (failed counters: one `unexpected_reset` and one `aborted` per side). The plan seed makes the 72 draws of the test identical in every run, and the old range of 200 ports drew 9 ports twice. A temporary log of bound and connected data ports showed the port with two connects before one accept. The range is now 53000 to 62999, where the 72 draws are all different, and the mutex is gone. With the three tests in parallel and three busy loops beside them, the old range failed 11 of 69 runs, the new one 0 of 44. The ignored repeat tests run each test 50 times side by side: 150 rounds passed under three busy loops, 281 s.
3. Checks: `cargo fmt -p fps-agent -- --check` and `cargo clippy -p fps-agent --all-targets -- -D warnings` pass.

4. CI proof. Commit baf1495 (the temporary commits removed the `#[ignore]` of the repeat tests and ignored the single tests, because a test and its repeat share ports and a plan file): job `rust` 130041905692 passed, with `repeat_the_ftp_test`, `repeat_the_ssh_test`, and `repeat_the_mail_test` ok (50 rounds each, side by side, 3 min 19 s) and `repeat_stopping_a_capture_at_rest` ok (50 rounds, 102 s). The first attempt of the temporary commit failed because the single tests and the repeat tests ran together on the same ports, which is why they are ignored there. The temporary commits are reverted.

## Remaining

The full body of the capture test 50 times (21 minutes) needs a workflow step that the coordinator adds, in the PR description.

## Next action

Review.

## Blockers and open questions

none

## Decisions taken in this package

- The capture test stops at rest instead of changing the capture path, because the capture crate is outside the paths of this package, and a session that the stop cuts off is not recorded in any design that keeps one file per session. The counter that mixes it with queue overflow is a follow up (see the log entry).
- The ftp data port range is a property of the seeded plan. Any change to the scenario, the seed, or the rate changes the draws, and the repeat tests catch a collision.

## Pitfalls

The sandbox has 2 CPUs. Reproduce timing flakes with busy loops (`while :; do :; done`) beside the test. The worktree guard refuses `source`, heredocs next to `git`, and long compound shell commands: write loops to a script file in the scratchpad.

## Verify the current state

```bash
CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-agent --test run_control --test first_party
CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-agent --test first_party -- --ignored repeat
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T17:55Z | claude-048417 | claim | none | claimed #389 with 4 steps |
| 2026-11-10T18:14Z | claude-048417 | checkpoint | f6e53de | capture flake fixed in the test |
| 2026-11-10T18:23Z | claude-048417 | checkpoint | b942d4b | ftp cause found, mutex removed, repeat tests added |
| 2026-11-10T18:50Z | claude-048417 | finish | see branch | repeat tests passed in CI, temporary commits reverted, ready for review |
