# Handoff TRN-23 Loss recovery and NewReno in the own stack

| Field | Value |
|---|---|
| Issue | #624 |
| Branch | `wp/TRN-23` |
| Pull request | #645 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-eae373, lease until 2026-11-18T05:08:24Z |
| Last verified commit | 46a1022 (CI green on #645) |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The own stack (`engine/fps-tcp-own`) recovers from loss: an RFC 6298 RTT estimator and RTO with Karn's rule, fast retransmit on three duplicate ACKs, an out of order receive queue, and NewReno (RFC 5681, RFC 6582). Idle sockets keep their flat cost. At 1 percent loss and 20 ms of delay every transfer completes and the own goodput is at least 50 percent of the kernel on the same link.

## Plan

- [x] 1. Pure mechanisms in `recovery.rs` (estimator, NewReno, out of order queue) with unit tests
- [x] 2. Wire them into the state machine (`host.rs`), per-connection state in a box that exists only while data is in flight
- [x] 3. Host tests for loss recovery over the lossy link (`engine/fps-tcp/tests/lossy.rs`)
- [x] 4. Measure the loss workload and the 1M scale row, commit results with host.txt
- [x] 5. Docs: Transport page, log entry, bench notes
- [x] 6. Checks, PR, CI, review

## Done and verified

- `cargo test -p fps-tcp-own -p fps-tcp` passes (72 unit tests in the lib incl. 14 host level recovery tests, 24 integration); clippy -D warnings clean.
- Sandbox loss run (own), committed in `engine/fps-tcp/results/sandbox-2vcpu-trn23/`: 0% 12.0, 0.1% 7.5, 1% 3.12 Mbit/s, all complete (was 12.2, 8.0, 1.8 with one transfer unfinished). 1M scale row: +8 B per socket against main in the same session, probe rate within 4 percent.
- cargo deny, plan.py check, render --check, lint, fmt, clippy pass.

## Remaining

CI result on PR #645, then review. The kernel comparison at 1 percent is made by the coordinator on the hosted runner (job=loss).

## Next action

Wait for CI on #645; fix failures inside the package paths.

## Blockers and open questions

none

## Decisions taken in this package

- Recovery state is `Option<Box<Recovery>>` in `Conn` (8 bytes idle), dropped when nothing is in flight, nothing waits to be sent, and nothing is held out of order.
- Slow start grows by the bytes acknowledged (the stack acks a batch with one ACK), full ACK ends recovery at ssthresh, Limited Transmit on dupacks 1 and 2, `recover` holds snd_max.
- The receiver now sends a window update when a read opened the window by a segment since the last advertised window (`adv_wnd`, 2 bytes); the old rule only fired when the window had been nearly closed, which halved the 0% goodput once the congestion window kept the flight below the window.

## Pitfalls

- A test helper that counts data segments must read the TCP header length: the SYN carries an MSS option.

## Verify the current state

```bash
cargo test -p fps-tcp-own
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T04:08Z | claude-eae373 | claim | none | claimed #624, base origin/main b3c4b29 |
| 2026-11-18T05:05Z | claude-eae373 | checkpoint | e248c3a | mechanisms, host tests, clippy clean |
| 2026-11-18T04:25Z | claude-eae373 | pr | ed535f4 | opened #645, waiting for CI |
| 2026-11-18T04:50Z | claude-eae373 | review | 46a1022 | CI green, ready for review |
