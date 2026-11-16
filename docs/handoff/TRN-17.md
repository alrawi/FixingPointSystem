# Handoff TRN-17 TCP stack bench harness and smoltcp behind SessionIo

| Field | Value |
|---|---|
| Issue | #571 |
| Branch | `wp/TRN-17` |
| Pull request | none yet |
| Status | in progress |
| Progress | 50% (3 of 6 plan steps) |
| Current holder | claude-7e8c87, lease until 2026-11-17T00:23:35Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Build `engine/fps-tcp/`, a harness that drives a user space TCP stack behind `SessionIo` and measures sessions per second, throughput, and socket scaling toward 1,000,000 sockets, with smoltcp as the first stack. TRN-18 adds an own stack on the same harness, and TRN-10 compares the results in decision record 0014. Results are committed for the CI runner, and a host test gives the fat-nuc run.

## Plan

- [x] 1. Claim, branch, handoff, and a claim commit.
- [x] 2. Crate skeleton, smoltcp license check with cargo deny.
- [x] 3. The `TcpStack` seam, the smoltcp adapter, and the `SessionIo` stream.
- [ ] 4. The three measures (sessions per second, throughput, socket scaling).
- [ ] 5. The bench binary with plain argv, tests, and committed CI results.
- [ ] 6. Docs, log entry, pull request, and CI.

## Done and verified

- `cargo test -p fps-tcp` passes (5 unit tests, 9 adapter tests through `SessionIo`).
- `cargo clippy -p fps-tcp --all-targets -- -D warnings` is clean.
- `cargo deny check` (v0.21.0): bans, sources, and advisories pass. Licenses FAIL for `smoltcp 0.14.0` and `managed 0.8.0`, both `0BSD`, which `deny.toml` does not list. Loosening `deny.toml` belongs to the owner under D13, so this package does not edit it.

## Remaining

The three measures, the bench binary, committed results, docs, and the pull request.

## Next action

Write `src/bench.rs` with the rate, throughput, and scale workloads.

## Blockers and open questions

The owner must allow the license `0BSD` in `deny.toml` (or an exception for `smoltcp` and `managed`), or `cargo deny check` fails in CI.

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
git log --oneline -3
```

## Host test

Not written yet.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16 | claude-7e8c87 | claim | none | Claimed #571 with 6 steps |
