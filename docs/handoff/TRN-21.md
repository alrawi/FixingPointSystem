# Handoff TRN-21 The own TCP stack in a crate of its own

| Field | Value |
|---|---|
| Issue | #621 |
| Branch | `wp/TRN-21` |
| Pull request | #638 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-b1c951, lease until 2026-11-18T02:08:13Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Move the own stack of TRN-18 (`engine/fps-tcp/src/own/` and its tests) into a new crate `engine/fps-tcp-own` with no smoltcp dependency (decision record 0014, item 2). `fps-tcp` keeps the harness, the `TcpStack` adapter of the own stack, and the smoltcp and kernel adapters, and depends on the new crate. No behavior change.

## Plan

- [x] 1. Create `fps-tcp-own` with the stack, the clock, and the component tests; `cargo tree` shows no smoltcp.
- [x] 2. Reduce `fps-tcp` to the adapter and re-exports; tests, clippy, fmt pass for both crates.
- [x] 3. Run tcp_bench for the own stack (rate and a socket-scaling row) and commit the run under `engine/fps-tcp/results/` with host.txt.
- [x] 4. Log entry, README and component page updates, plan checks, `cargo deny check`, PR and CI.

## Done and verified

- Steps 1 and 2: `cargo test -p fps-tcp-own -p fps-tcp` passes (fps-tcp-own 9 unit + 22 component tests; fps-tcp 25 unit + 8 kernel + 1 own_workloads + 9 smol + 5 workloads). `cargo tree -p fps-tcp-own | grep -c smoltcp` prints 0. `cargo clippy -p fps-tcp -p fps-tcp-own --all-targets -- -D warnings` and `cargo fmt --check` are clean.

- Step 4: PR #638, CI on dbe3c4d: 62 check runs, all completed success.
- Step 3: results in `engine/fps-tcp/results/sandbox-2vcpu-trn21/` (A/B of the origin/main binary and this branch: rate 33.5k-35.5k vs 34.5k-35.5k sessions/s; scale 1,000,629 sockets, 228 B each, 56.1k vs 55.7k probe sessions/s). `cargo deny check` ok. plan.py check, render --check, lint ok.

## Remaining

none

## Next action

Coordinator review and merge.

## Blockers and open questions

none

## Decisions taken in this package

- `OwnStack` gets inherent `listen`, `connect`, `accept`, `drive`, `stats` (type `OwnStats`); the `TcpStack` impl in `fps-tcp/src/own.rs` delegates to them. The stack code is unchanged. The orphan rule allows the impl because `TcpStack` is local to `fps-tcp`.
- The clocks moved with the stack (the stack reads them) and `fps-tcp` re-exports the module as `fps_tcp::clock`.
- `Cargo.lock` is part of the PR (new package).

- The workspace members glob `engine/*` already covers the new crate, so the root `Cargo.toml` needs no edit.

## Pitfalls

none yet

## Verify the current state

```bash
cargo tree -p fps-tcp-own | grep -c smoltcp   # expect 0
cargo test -p fps-tcp-own -p fps-tcp
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-b1c951 | claim | 8bd0991 | claimed #621 |
| 2026-11-18 | claude-b1c951 | checkpoint | 3321862 | steps 1 and 2 done, docs and log entry added |
| 2026-11-18 | claude-b1c951 | checkpoint | see git log | step 3 done, bench results committed |
| 2026-11-18 | claude-b1c951 | review | dbe3c4d | PR #638, CI green, ready for review |
