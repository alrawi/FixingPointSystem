# Handoff TRN-25 CUBIC in the own stack

| Field | Value |
|---|---|
| Issue | #626 |
| Branch | `wp/TRN-25` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 20% (1 of 5 plan steps) |
| Current holder | claude-9a5394, see the status card on the issue |
| Last verified commit | none |

## Goal

CUBIC (RFC 9438) next to the NewReno of TRN-23 in the own stack, selectable in the stack configuration (`OwnConfig::congestion_control`, `tcp_bench --own-cc newreno|cubic`), with the state inside the in-flight `Recovery` box. Acceptance: four flows over a 1 Gbps bottleneck with 20 ms of delay reach a Jain index of at least 0.9 and 80 percent of four kernel flows; one flow on the fat path reaches 80 percent of the default (CUBIC) kernel row.

## Plan

- [x] 1. `cubic.rs` (port of smoltcp 0.14.0 CUBIC), the choice in `OwnConfig`, the wiring in `recovery.rs` and `host.rs`, unit and host tests.
- [ ] 2. `tcp_bench --own-cc`, the multi flow workload (`--loss-flows`, `--loss-profile bottleneck`), kernel peers for N flows, report rows.
- [ ] 3. Sandbox measurements (own newreno and cubic, smoltcp), bytes per socket on the 1M scale row, results folder.
- [ ] 4. Wiki: component page, log entry, plan entry check; checks (test, clippy, fmt, deny, plan.py).
- [ ] 5. Pull request, CI, review.

## Done and verified

- Step 1: `cargo test -p fps-tcp-own` passes (153 unit tests, 24 integration).

## Remaining

Steps 2 to 5.

## Next action

Add `--own-cc` and the multi flow loss workload to `engine/fps-tcp`.

## Blockers and open questions

none

## Decisions taken in this package

- CUBIC is the default, like the Linux default. The existing NewReno host tests pin `CongestionControl::NewReno` in `plain()`.
- The epoch of a stage starts at the first acknowledgment after an event (RFC 9438 section 4.2) and `K` uses the window at that moment; smoltcp starts it at the event.
- The slow start threshold is 0.7 of the flight (as smoltcp), `W_max` comes from the window with fast convergence.
- No HyStart (RFC 9406): slow start leaves on loss, as in NewReno.

## Pitfalls

The sandbox guard refuses heredocs and `source` in the same command as git; write files with the editor tools and run python scripts from the scratch folder.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-TRN-25 CARGO_BUILD_JOBS=1 cargo test -p fps-tcp-own
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-9a5394 | claim | none | claimed #626 |
