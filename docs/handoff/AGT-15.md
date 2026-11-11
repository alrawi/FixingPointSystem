# Handoff AGT-15 Stabilize the agent enrollment test

| Field | Value |
|---|---|
| Issue | #337 |
| Branch | `wp/AGT-15` |
| Pull request | #474 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-bb4766, lease until 2026-11-11T19:41:38Z (renewed at each checkpoint) |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`engine/fps-agent/tests/mtls.rs` `an_agent_enrolls_with_a_token_registers_and_renews` failed once at its line 550 assertion and passed on re-run on 2026-11-09. Find and fix the race without adding sleeps, and explain the cause in the PR. The test must pass 200 times in a row locally and in a temporary CI step, which is then removed.

## Plan

- [x] 1. Reproduce: shorten the fake endpoint's renewal delay and see the line 550 assertion fail.
- [x] 2. Fix the test so the renewal is gated by the test, not by wall-clock time.
- [x] 3. Run the test 200 times locally.
- [x] 4. Temporary CI loop step, then remove it; open the PR.

## Done and verified

- Reproduced: with the endpoint's renewal delay cut to 1 ms the old test fails at `mtls.rs:550` (`names` was three entries).
- Fixed in `engine/fps-agent/tests/mtls.rs`: `/renew` is held until the test releases it, and a renewal answer always carries the one hour delay.
- 200 of 200 passes locally (plus 100 of 100 under four busy loops); `cargo test -p fps-agent --test mtls` 19 passed; fmt and clippy clean; plan check, render --check, lint ok.
- Temporary CI loop (200 runs on the runner, run 43078716883) passed at cd1c412; the workflow file is removed in the next commit. All checks of the PR were green.

## Remaining

Nothing.

## Next action

Coordinator reviews and merges PR #474.

## Blockers and open questions

none

## Decisions taken in this package

Cause (confirmed): the fake enrollment endpoint sets `renew_at` 1500 ms after the enrollment. On a loaded runner the session can take longer than that to open, so the renewal's CSR name is pushed to `names` before line 550 reads it, and the list holds two entries instead of one.

## Pitfalls

Test binaries take `--exact <full name>`; a second filter makes the run match nothing.

## Verify the current state

```bash
cargo test -p fps-agent --test mtls   # 19 passed
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-11T18:11Z | claude-bb4766 | claim | none | Claimed #337 |
| 2026-11-11T18:40Z | claude-bb4766 | checkpoint | see branch | fix verified locally, temporary CI loop added |
| 2026-11-11T19:00Z | claude-bb4766 | finish | see branch | CI green, loop workflow removed, ready for review |
