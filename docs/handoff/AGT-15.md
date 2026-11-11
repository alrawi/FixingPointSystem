# Handoff AGT-15 Stabilize the agent enrollment test

| Field | Value |
|---|---|
| Issue | #337 |
| Branch | `wp/AGT-15` |
| Pull request | not yet opened |
| Status | in progress |
| Progress | 10% (0 of 4 plan steps) |
| Current holder | claude-bb4766, lease until 2026-11-11T19:41:38Z (renewed at each checkpoint) |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`engine/fps-agent/tests/mtls.rs` `an_agent_enrolls_with_a_token_registers_and_renews` failed once at its line 550 assertion and passed on re-run on 2026-11-09. Find and fix the race without adding sleeps, and explain the cause in the PR. The test must pass 200 times in a row locally and in a temporary CI step, which is then removed.

## Plan

- [ ] 1. Reproduce: shorten the fake endpoint's renewal delay and see the line 550 assertion fail.
- [ ] 2. Fix the test so the renewal is gated by the test, not by wall-clock time.
- [ ] 3. Run the test 200 times locally.
- [ ] 4. Temporary CI loop step, then remove it; open the PR.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Build `fps-agent` tests and reproduce.

## Blockers and open questions

none

## Decisions taken in this package

Hypothesis: the fake enrollment endpoint sets `renew_at` 1500 ms after the enrollment. On a loaded runner the session can take longer than that to open, so the renewal's CSR name is pushed to `names` before line 550 reads it, and the list holds two entries instead of one.

## Pitfalls

none yet

## Verify the current state

```bash
source /tmp/.../.fps_env-AGT-15; cargo test -p fps-agent --test mtls an_agent_enrolls
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-11T18:11Z | claude-bb4766 | claim | none | Claimed #337 |
