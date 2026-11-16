# Handoff TRN-11 AF_XDP packet input and output

| Field | Value |
|---|---|
| Issue | #562 |
| Branch | `wp/TRN-11` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-7c41e9, lease until 2026-11-17T00:23:48Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Build the crate `engine/fps-backend-afxdp` with the UMEM, fill, completion, receive, and transmit rings, batching per queue, zero copy mode with a fallback to copy mode, per queue workers, and a published frame trait with batched receive and transmit per queue. Defaults come from spike S3. Acceptance: AF_XDP on a veth pair in the namespace harness passes frames both ways in CI, falls back to copy mode, and reads the bind mode from the socket. The KVM rate test is TRN-19 and not part of this package.

## Plan

- [ ] 1. Claim, handoff file, and crate skeleton with the frame trait and an in memory fake
- [ ] 2. UMEM, the four rings, and the `sys` module for the socket and mmap calls
- [ ] 3. XDP program loader and bind with zero copy, copy fallback, and bind mode read back
- [ ] 4. Per queue workers and batched receive and transmit
- [ ] 5. Veth test in the namespace harness and the Host test script
- [ ] 6. Wiki ingest, log entry, checks, pull request, and CI

## Done and verified

Nothing yet.

## Remaining

Everything in the plan.

## Next action

Write the crate skeleton and the frame trait.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-backend-afxdp
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16T22:55Z | claude-7c41e9 | claim | none | Claimed #562 with card 6931824639 |
