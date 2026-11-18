# Handoff TRN-26 Interop of the own stack with smoltcp and the kernel

| Field | Value |
|---|---|
| Issue | #627 |
| Branch | `wp/TRN-26` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-7c2e91, lease until 2026-11-18T09:58:49Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Conformance and interop tests in the harness (`engine/fps-tcp/`): the own stack against smoltcp over the lossy link in memory (both directions, 0 and 1 percent loss, 20 ms per direction), against the kernel stack over a veth pair with netem and an AF_XDP queue (both directions, 1 percent loss), option negotiation checks visible on the wire, and a matrix in `Transport.md`. Acceptance: every pairing completes the loss workload at 1 percent loss in both directions, and the interop tests pass in CI.

## Plan

- [ ] 1. Claim, handoff, and a pattern checked loss transfer in `workload.rs`.
- [ ] 2. A wire tap that reads TCP options with the smoltcp parser, and the smoltcp adapter with one external side.
- [ ] 3. The in memory pairings (own and smoltcp, both directions) and `tests/interop_smol.rs`.
- [ ] 4. The kernel pairing: three namespaces, AF_XDP queue on the own arm, peer roles, and `tests/interop_kernel.rs`.
- [ ] 5. The matrix in `Transport.md`, the log entry, and the checks.
- [ ] 6. Pull request, CI, review.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Step 1.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-tcp
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T08:29Z | claude-7c2e91 | claim | none | lease 30 minutes |
