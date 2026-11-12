# Handoff ENG-28 The runtime instance test flakes of batch 29

| Field | Value |
|---|---|
| Issue | #479 |
| Branch | `wp/ENG-28` |
| Pull request | #0 |
| Status | in progress |
| Progress | 50% (2 of 4 plan steps) |
| Current holder | claude-960dd7, lease until 2026-11-12T04:26:13Z |
| Last verified commit | none |

## Goal

Two tests of `engine/fps-runtime/tests/instances.rs` failed once each in batch 29 on pull requests that did not touch the runtime: `mixed_session_runs_with_one_data_connection_per_transfer` (line 318, message `one SYN per connection`, job 130718999945 of PR #470) and `concurrent_instances_that_draw_the_same_data_port_all_succeed` (line 713, `("side", "client")`, left 16, right 32, job 130740042708 of PR #475). Find the cause of each, fix it in the test or the runtime, and show 200 consecutive passes of both tests on the hosted runner with a temporary CI loop.

## Plan

- [x] 1. Read both failures and find the cause of each
- [x] 2. Fix the causes and reproduce locally where the sandbox allows it (failure 1 only)
- [ ] 3. Temporary CI loop of 200 runs of both tests, then remove the loop
- [ ] 4. Ingest (component page, log entry) and open the pull request

## Done and verified

1. Failure 1 (line 318) reproduced locally: 1 failure in 120 runs of the whole instances binary with one busy loop. Cause: the test treats a repeated (source host, source port) pair among the client SYNs as a second SYN of one connection, but the client sockets bind with `IP_BIND_ADDRESS_NO_PORT`, so the kernel may give one source host the same port for two connections to different destination ports. A diagnostic run showed `127.61.0.4:40204` as the source of two SYNs, one to the control port and one to data port 52486. The set now holds the four tuple.
2. Failure 2 (line 713, 16 of 32) did not reproduce in about 600 local runs, with and without busy loops. The log of job 130740042708 shows the test took 6.1 s against about 1.3 s in a passing run, which fits one 5 s timeout (`connect_timeout` or an `expect` of 5000 ms) in the client. The test now names the failed series before it counts successes, so a repeat names the reason.

## Remaining

Temporary CI loop (`.github/workflows/eng-28-loop.yml`, delete before review), cause of failure 2, ingest, pull request.

## Next action

Push the branch and read the loop job of `eng-28-loop`; a failure prints `failed_series` for failure 2.

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
| 2026-11-12T02:58:00Z | claude-960dd7 | claim | b2028d2 | Claimed issue #479 with card 6813013946 |
| 2026-11-12T03:46:00Z | claude-960dd7 | checkpoint | see git log | Fix of failure 1, diagnostics for failure 2, temporary loop workflow |
