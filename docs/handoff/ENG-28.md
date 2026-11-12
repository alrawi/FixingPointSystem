# Handoff ENG-28 The runtime instance test flakes of batch 29

| Field | Value |
|---|---|
| Issue | #479 |
| Branch | `wp/ENG-28` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-960dd7, lease until 2026-11-12T04:26:13Z |
| Last verified commit | none |

## Goal

Two tests of `engine/fps-runtime/tests/instances.rs` failed once each in batch 29 on pull requests that did not touch the runtime: `mixed_session_runs_with_one_data_connection_per_transfer` (line 318, message `one SYN per connection`, job 130718999945 of PR #470) and `concurrent_instances_that_draw_the_same_data_port_all_succeed` (line 713, `("side", "client")`, left 16, right 32, job 130740042708 of PR #475). Find the cause of each, fix it in the test or the runtime, and show 200 consecutive passes of both tests on the hosted runner with a temporary CI loop.

## Plan

- [ ] 1. Read both failures and find the cause of each
- [ ] 2. Fix the causes and reproduce locally where the sandbox allows it
- [ ] 3. Temporary CI loop of 200 runs of both tests, then remove the loop
- [ ] 4. Ingest (component page, log entry) and open the pull request

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read `engine/fps-runtime/tests/instances.rs` around lines 300 to 330 and 690 to 740 and compare with the job logs.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

The jobs 130718999945 and 130740042708 show only the failing tests in the last lines of the log. Read them with `gh api repos/alrawi/FixingPointSystem/actions/jobs/<id>/logs`.

## Verify the current state

```bash
git fetch origin wp/ENG-28 && git log --oneline origin/main..origin/wp/ENG-28
cargo test -p fps-runtime --test instances
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T02:58:00Z | claude-960dd7 | claim | none | Claimed issue #479 with card 6813013946 |
