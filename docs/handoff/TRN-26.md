# Handoff TRN-26 Interop of the own stack with smoltcp and the kernel

| Field | Value |
|---|---|
| Issue | #627 |
| Branch | `wp/TRN-26` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 50% (3 of 6 plan steps) |
| Current holder | claude-7c2e91, lease until 2026-11-18T09:58:49Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Conformance and interop tests in the harness (`engine/fps-tcp/`): the own stack against smoltcp over the lossy link in memory (both directions, 0 and 1 percent loss, 20 ms per direction), against the kernel stack over a veth pair with netem and an AF_XDP queue (both directions, 1 percent loss), option negotiation checks visible on the wire, and a matrix in `Transport.md`. Acceptance: every pairing completes the loss workload at 1 percent loss in both directions, and the interop tests pass in CI.

## Plan

- [x] 1. Claim, handoff, and a pattern checked loss transfer in `workload.rs`.
- [x] 2. A wire tap that reads TCP options with the smoltcp parser, and the smoltcp adapter with one external side.
- [x] 3. The in memory pairings (own and smoltcp, both directions) and `tests/interop_smol.rs`.
- [ ] 4. The kernel pairing: three namespaces, AF_XDP queue on the own arm, peer roles, and `tests/interop_kernel.rs`.
- [ ] 5. The matrix in `Transport.md`, the log entry, and the checks.
- [ ] 6. Pull request, CI, review.

## Done and verified

- Steps 1 to 3: `workload.rs` has `LossConfig::verify` (position dependent pattern, checked by the receiver), `wiretap.rs` reads options with the smoltcp parser, `smol.rs` has `SmolStack::bridged` (one smoltcp host behind an `IpBridge`), `interop.rs` has `InteropStack` (own and smoltcp over the impaired link with Ethernet framing). `cargo test -p fps-tcp --test interop_smol` passes 8 tests, five runs in a row (about 8 s each run).

## Remaining

Steps 4 to 6: the kernel pairing, the matrix in Transport.md, the log entry, the checks, the pull request.

## Next action

Step 4: write `src/interop_kernel.rs` (namespaces, AF_XDP queue, peer roles) and `tests/interop_kernel.rs`.

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
| 2026-11-18T08:58Z | claude-7c2e91 | checkpoint | see git | steps 1 to 3 done |
