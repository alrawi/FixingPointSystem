# Handoff TRN-23 Loss recovery and NewReno in the own stack

| Field | Value |
|---|---|
| Issue | #624 |
| Branch | `wp/TRN-23` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-eae373, lease until 2026-11-18T05:08:24Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The own stack (`engine/fps-tcp-own`) recovers from loss: an RFC 6298 RTT estimator and RTO with Karn's rule, fast retransmit on three duplicate ACKs, an out of order receive queue, and NewReno (RFC 5681, RFC 6582). Idle sockets keep their flat cost. At 1 percent loss and 20 ms of delay every transfer completes and the own goodput is at least 50 percent of the kernel on the same link.

## Plan

- [ ] 1. Pure mechanisms in `recovery.rs` (estimator, NewReno, out of order queue) with unit tests
- [ ] 2. Wire them into the state machine (`host.rs`), per-connection state in a box that exists only while data is in flight
- [ ] 3. Host tests for loss recovery over the lossy link (`engine/fps-tcp/tests/lossy.rs`)
- [ ] 4. Measure the loss workload and the 1M scale row, commit results with host.txt
- [ ] 5. Docs: Transport page, log entry, bench notes
- [ ] 6. Checks, PR, CI, review

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write `engine/fps-tcp-own/src/recovery.rs`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-tcp-own
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T04:08Z | claude-eae373 | claim | none | claimed #624, base origin/main b3c4b29 |
