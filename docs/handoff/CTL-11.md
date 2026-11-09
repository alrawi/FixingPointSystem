# Handoff CTL-11 Module signing and signed index

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #163 |
| Branch | `wp/CTL-11` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-ctl11-a7c3e1, lease until 2026-11-09T04:07:11Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ed25519 signing and verification for module archives (the signature covers the digest string, as ModuleRef.signature in CON-05 says) and for a canonical registry index, key ids derived from public keys, and a trust store with active, retired (until a cutoff), and revoked keys. The package is `controller/internal/registry/signing/`, standard library only. Install and the agent do not use it yet.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim, branch, handoff
- [ ] 2. Keys, trust store, digest signing and archive digest, with tests
- [ ] 3. Canonical index and its signature, with tests; make targets
- [ ] 4. Controller page, log entry, checks, PR, CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Step 2.

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
| 2026-11-08T22:40Z | claude-ctl11-a7c3e1 | claim | none | Claimed issue #163, branch from origin/main 82a5953 |
