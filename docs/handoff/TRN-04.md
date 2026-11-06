# Handoff TRN-04 TLS transport module

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #159 |
| Branch | `wp/TRN-04` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-t04a1f, lease until 2026-11-06T22:35:32Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Wrap sessions in TLS 1.2 and 1.3 with rustls, with client profiles (SNI, ALPN, versions, groups, cipher order), per run server certificates from a generated authority, resumption, and NSS key log export. Acceptance: handshake rates within 10% of the S2 spike, and tshark decrypts a sample capture with the exported keys.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Crate skeleton, authority, client and server profiles, resumption
- [ ] 2. Async session wrapper over SessionIo, handshake reports, key log
- [ ] 3. Unit and loopback tests for versions, ALPN, SNI, groups, ciphers, resumption
- [ ] 4. Handshake rate benchmark against the S2 results
- [ ] 5. Capture and key log test with tshark check
- [ ] 6. README, wiki page, log entry, checks, pull request

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Write exactly one concrete step that the next agent can start immediately.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T22:25Z | claude-t04a1f | claim | | Claimed #159 with 6 steps |
