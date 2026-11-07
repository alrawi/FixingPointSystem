# Handoff ENG-12 Session sampling and capture

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #157 |
| Branch | `wp/ENG-12` |
| Pull request | #0 |
| Status | in progress |
| Progress | 83% (5 of 6 plan steps) |
| Current holder | claude-eng12a, lease until 2026-11-07T18:52:38Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Mirror a configurable, seeded sample of sessions into a bounded pcapng ring with TLS key logs, in the new crate `engine/fps-capture`. Acceptance: a 1% sample of 10,000 sessions yields between 70 and 130 captured sessions.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Sampler and pcapng writer
- [x] 2. Frame synthesis (Ethernet, IPv4, IPv6, TCP, UDP)
- [x] 3. Session tap and recorder
- [x] 4. Ring with rotation and key logs
- [x] 5. Tests, tshark check
- [ ] 6. Wiki, log entry, PR and CI

## Done and verified

Steps 1 to 5. `cargo test -p fps-capture` passes 17 unit, 14 component, and 1 doc test. With tshark 4.2.2 installed, `FPS_REQUIRE_CAPTURE_TOOLS=1 cargo test -p fps-capture --test capture` passes all 14, including the tshark parse and the TLS 1.2 and 1.3 decryption from embedded and sidecar keys. `cargo clippy -p fps-capture --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Open the pull request and wait for CI. Known gaps are in the last section of the Engine page, which are the runtime wiring, the missing AF_PACKET ring and pcap filter, and the digest of a multi file ring.

## Next action

Open the pull request, wait for CI, then set ready for review.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- Frames are synthesized from socket level bytes through tap wrappers, with no AF_PACKET ring, as the coordinator instructed. This keeps the crate free of unsafe code.
- The sampler is a stateless hash of seed and session id, so workers share nothing.
- Frame synthesis and file writing run on a writer thread, so a sampled session costs its worker one copy.
- Key logs are per session, and the default mode writes both an embedded decryption secrets block and a sidecar file.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
cargo test -p fps-capture   # tshark tests skip without tshark
FPS_REQUIRE_CAPTURE_TOOLS=1 cargo test -p fps-capture --test capture
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-07T17:23:00Z | claude-eng12a | claim | none | Claimed #157 |
| 2026-11-07T18:00:00Z | claude-eng12a | checkpoint | see PR | Crate, tests, wiki, log entry |
