# Handoff TRN-17 TCP stack bench harness and smoltcp behind SessionIo

| Field | Value |
|---|---|
| Issue | #571 |
| Branch | `wp/TRN-17` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-7e8c87, lease until 2026-11-17T00:23:35Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Build `engine/fps-tcp/`, a harness that drives a user space TCP stack behind `SessionIo` and measures sessions per second, throughput, and socket scaling toward 1,000,000 sockets, with smoltcp as the first stack. TRN-18 adds an own stack on the same harness, and TRN-10 compares the results in decision record 0014. Results are committed for the CI runner, and a host test gives the fat-nuc run.

## Plan

- [ ] 1. Claim, branch, handoff, and a claim commit.
- [ ] 2. Crate skeleton, smoltcp license check with cargo deny.
- [ ] 3. The `TcpStack` seam, the smoltcp adapter, and the `SessionIo` stream.
- [ ] 4. The three measures (sessions per second, throughput, socket scaling).
- [ ] 5. The bench binary with plain argv, tests, and committed CI results.
- [ ] 6. Docs, log entry, pull request, and CI.

## Done and verified

Nothing yet.

## Remaining

Everything after step 1.

## Next action

Create `engine/fps-tcp/Cargo.toml` and run `cargo deny check` with smoltcp added.

## Blockers and open questions

none

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
