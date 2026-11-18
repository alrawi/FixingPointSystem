# Handoff TRN-21 The own TCP stack in a crate of its own

| Field | Value |
|---|---|
| Issue | #621 |
| Branch | `wp/TRN-21` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-b1c951, lease until 2026-11-18T02:08:13Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Move the own stack of TRN-18 (`engine/fps-tcp/src/own/` and its tests) into a new crate `engine/fps-tcp-own` with no smoltcp dependency (decision record 0014, item 2). `fps-tcp` keeps the harness, the `TcpStack` adapter of the own stack, and the smoltcp and kernel adapters, and depends on the new crate. No behavior change.

## Plan

- [ ] 1. Create `fps-tcp-own` with the stack, the clock, and the component tests; `cargo tree` shows no smoltcp.
- [ ] 2. Reduce `fps-tcp` to the adapter and re-exports; tests, clippy, fmt pass for both crates.
- [ ] 3. Run tcp_bench for the own stack (rate and a socket-scaling row) and commit the run under `engine/fps-tcp/results/` with host.txt.
- [ ] 4. Log entry, README and component page updates, plan checks, `cargo deny check`, PR and CI.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Step 1.

## Blockers and open questions

none

## Decisions taken in this package

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
| 2026-11-18 | claude-b1c951 | claim | none | claimed #621 |
