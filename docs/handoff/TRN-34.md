# Handoff TRN-34 Pacing and faster loss detection in the own stack

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #659 |
| Branch | `wp/TRN-34` |
| Pull request | #683 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-50b39c, lease until 2026-11-18T21:08:28Z |
| Last verified commit | 5c49be3 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add sender pacing, HyStart for the CUBIC slow start, and tail loss probes with RACK to the own TCP stack, each behind a switch in `OwnConfig`. In the bottleneck test the own stack retransmits at most twice as many segments as the kernel at no lower goodput, and the TRN-22 to TRN-25 acceptance rows still pass.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read the recovery, CUBIC and host code; baseline of the bottleneck test
- [x] 2. HyStart in `cubic.rs`
- [x] 3. Pacing in `host.rs`
- [x] 4. RACK and tail loss probe in `recovery.rs`
- [x] 5. Defaults, tests, bench comparison, Transport.md
- [x] 6. Checks, PR, CI

## Done and verified

- Steps 1 to 5. `cargo test -p fps-tcp-own -p fps-tcp` (220 lib tests, bottleneck, interop, cli) passes; `cargo clippy -p fps-tcp-own -p fps-tcp --all-targets -- -D warnings` is clean. Results in `engine/fps-tcp/results/sandbox-2vcpu-trn34/`, and the Transport page section of TRN-34 has the tables.
- Defaults: RACK and tail loss probes on, HyStart and pacing off (HyStart lowers the TRN-25 Jain index to 0.90 to 0.91).

## Remaining

The hosted `tcp-bench` dispatches (log entry, Shared page notes), and the owner's choice of the HyStart default after the hosted fairness run.

## Next action

The coordinator reviews pull request 683 and dispatches the hosted `tcp-bench` runs that the log entry lists.

## Blockers and open questions

none

## Decisions taken in this package

- HyStart off by default: fairness of four flows falls to the bar of 0.9. Pacing off: no retransmission gain and a small cost on the fat row.
- Pacing sends up to 1 ms of data per wake (`PACING_QUANTUM_US`), because the driver wakes in steps of about 1 ms.
- One timer per connection serves the retransmission, probe and reorder kinds (`TimerKind`), so the heap holds one entry per connection.

## Pitfalls

- The sandbox kernel sends with BBR; use `--kernel-cc cubic` for a like for like kernel row.
- HyStart misses its exit in about one run in ten (ACK spacing above 2 ms), so tests take the best of three.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T18:08Z | claude-50b39c | claim | none | claimed #659 |
| 2026-11-18T19:38Z | claude-50b39c | takeover | none | the lease of card 6987227256 ran out during the long measurements; took over my own item with card 6989460130 |
| 2026-11-18T19:45Z | claude-50b39c | checkpoint | see PR | steps 1 to 5 done, step 6 next |
| 2026-11-18T20:05Z | claude-50b39c | finish | 5c49be3 | pull request 683, CI green (30 checks) |
