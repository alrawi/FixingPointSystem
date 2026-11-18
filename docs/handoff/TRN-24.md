# Handoff TRN-24 TCP options for long fat paths in the own stack

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #625 |
| Branch | `wp/TRN-24` |
| Pull request | #0 |
| Status | in progress |
| Progress | 80% (5 of 6 plan steps) |
| Current holder | claude-e5c1d3, lease until 2026-11-18T06:18:38Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Window scaling, timestamps, SACK (RFC 2018 blocks, RFC 6675 recovery) and delayed ACK in the own stack, negotiated in the handshake with clean fallback. Acceptance: at 0.1 percent loss and 50 ms with BDP above 64 KB, own goodput is at least 80 percent of the kernel.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Option codec and negotiation (wire.rs), flags in a few bits
- [x] 2. Window scaling and timestamps (RTT samples, PAWS)
- [x] 3. SACK receiver blocks from the OOO queue
- [x] 4. SACK scoreboard, pipe, RFC 6675 recovery inside Recovery box
- [x] 5. Delayed ACK
- [ ] 6. tcp_bench loss configuration, results, log entry, component page

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

Step 6: tcp_bench loss configuration for 0.1 percent loss and 50 ms with BDP above 64 KB, sandbox runs of own and smoltcp, 1M scale row against main, log entry and Transport page, CI.

## Next action

Run `cargo test -p fps-tcp` (lossy tests with the new defaults), then add the loss configuration to `engine/fps-tcp/src/cli.rs` and run the bench.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-e5c1d3 | claim | - | claimed #625 |
