# Handoff TRN-25 CUBIC in the own stack

| Field | Value |
|---|---|
| Issue | #626 |
| Branch | `wp/TRN-25` |
| Pull request | opened, see the status card on the issue |
| Status | in progress |
| Progress | 90% (4 of 5 plan steps) |
| Current holder | claude-9a5394, see the status card on the issue |
| Last verified commit | see the session log |

## Goal

CUBIC (RFC 9438) next to the NewReno of TRN-23 in the own stack, selectable in the stack configuration (`OwnConfig::congestion_control`, `tcp_bench --own-cc newreno|cubic`), with the state inside the in-flight `Recovery` box. Acceptance: four flows over a 1 Gbps bottleneck with 20 ms of delay reach a Jain index of at least 0.9 and 80 percent of four kernel flows; one flow on the fat path reaches 80 percent of the default (CUBIC) kernel row.

## Plan

- [x] 1. `cubic.rs` (port of smoltcp 0.14.0 CUBIC), the choice in `OwnConfig`, the wiring in `recovery.rs` and `host.rs`, unit and host tests.
- [x] 2. `tcp_bench --own-cc`, the multi flow workload (`--loss-flows`, `--loss-profile bottleneck`), kernel peers for N flows, report rows.
- [x] 3. Sandbox measurements (own newreno and cubic, smoltcp), bytes per socket on the 1M scale row (263), results folder `engine/fps-tcp/results/sandbox-2vcpu-trn25/`.
- [x] 4. Wiki: Transport page, log entry; checks (test, clippy, fmt, deny, plan.py).
- [ ] 5. Pull request, CI, review.

## Done and verified

- `cargo test -p fps-tcp-own -p fps-tcp`, `cargo clippy --all-targets -D warnings`, `cargo fmt --check`, `cargo deny check`, `plan.py check`, `render --check`, `lint` pass.
- Sandbox: fat path own CUBIC 20.00 Mbit/s (mean of 6), 19.58 (mean of 24), NewReno 14.18 / 13.54, smoltcp 4.91. Four flows: 891 Mbit/s, Jain 0.941 (CUBIC), 0.962 (NewReno).

## Host test

The kernel rows need netem and run in the tcp-bench workflow (job=loss, hosted runner), not on fat-nuc. `loss_args` for the fat path: `--loss-profile fat --loss-transfers 6`. For four flows: `--loss-profile bottleneck --loss-transfers 3`.

## Remaining

CI result and review. The kernel rows of both acceptance items (hosted runner).

## Next action

Poll the check runs of the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- CUBIC is the default, like the Linux default. The existing NewReno host tests pin `CongestionControl::NewReno` in `plain()`.
- The epoch of a stage starts at the first acknowledgment after an event (RFC 9438 section 4.2) and `K` uses the window at that moment; smoltcp starts it at the event.
- The slow start threshold is 0.7 of the flight (as smoltcp), `W_max` comes from the window with fast convergence.
- No HyStart (RFC 9406) and no PRR (RFC 6937). If the hosted runner reads the fat row below 19.3 Mbit/s, PRR is the next candidate: each recovery now pauses the sender for about 0.3 round trips, and at 0.1 percent loss a recovery happens every 230 ms.
- The fairness run is a mode of the loss workload (`--loss-flows`), so the existing `loss_args` of the workflow carry it.

## Pitfalls

The sandbox guard refuses heredocs and `source` in the same command as git; write files with the editor tools and run python scripts from the scratch folder. The fat path row has a spread of 6 to 32 Mbit/s per transfer, so compare means of at least 6.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-TRN-25 CARGO_BUILD_JOBS=1 cargo test -p fps-tcp-own -p fps-tcp
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-9a5394 | claim | none | claimed #626 |
| 2026-11-18 | claude-9a5394 | checkpoint | 39d22b2 | CUBIC, config, tests |
| 2026-11-18 | claude-9a5394 | checkpoint | next commit | bench flags, fairness workload, results, wiki |
