# Handoff AGT-18 The run_control capture flake

| Field | Value |
|---|---|
| Issue | #389 |
| Branch | `wp/AGT-18` |
| Pull request | #0 |
| Status | in progress |
| Progress | 25% (1 of 4 plan steps) |
| Current holder | claude-048417, lease until 2026-11-10T18:55:02Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Find and fix the cause of the dropped packet in `a_fake_controller_changes_rates_and_reads_a_capture` (`engine/fps-agent/tests/run_control.rs`) and the cause of the lost ftp sessions when the tests of `tests/first_party.rs` run in parallel, then remove the mutex or justify it. Both tests pass 50 times in a row in CI, and the capture test still fails when a packet is really dropped.

## Plan

- [x] 1. Reproduce and fix the capture flake
- [ ] 2. Reproduce the ftp session loss and fix it or justify the mutex
- [ ] 3. Loop proof for CI (ignored loop tests), checks, wiki log entry, PR
- [ ] 4. CI green, review

## Done and verified

1. Cause of the capture flake: a session that is open when the capture stops is counted as dropped. Reproduced locally with three busy loops beside the test (3 of 4 runs failed with `dropped: 1` or `2`), and a temporary diagnostic showed every drop was `TrySendError::Disconnected`, never `Full`. The fix is in the test (stop at rest) plus a test that a really lost session is reported.

## Remaining

Steps 2 to 4.

## Next action

Reproduce the ftp loss with `first_party` tests running together, without the mutex.

## Blockers and open questions

none

## Decisions taken in this package

Local design choices are recorded as the steps finish.

## Pitfalls

The sandbox has 2 CPUs. Reproduce timing flakes with busy loops (`while :; do :; done`) beside the test.

## Verify the current state

```bash
CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-agent --test run_control
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T18:25Z | claude-048417 | claim | none | claimed #389 with 4 steps |
