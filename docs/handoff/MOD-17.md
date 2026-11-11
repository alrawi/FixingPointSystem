# Handoff MOD-17 LDAP module

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #184 |
| Branch | `wp/MOD-17` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-e41a7f, lease until 2026-11-11T05:14:04Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship ldap scenarios for bind, search, and unbind with BER encoding functions. The native crate `fps-module-ldap` holds a BER codec written in this crate, the LDAPv3 message encoders and readers, and a server side that answers binds and searches from a directory fixture. nDPI must classify at least 99% of the fixture sessions as LDAP.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim commit, BER codec and filter parser with unit tests
- [ ] 2. LDAP messages, directory model, search evaluation, the template functions and the catalog
- [ ] 3. Manifest, scenarios, and the interpreter, catalog, and transcript tests
- [ ] 4. fixture_peer, capture.sh, expected.yaml, and a capture in a namespace harness if root allows
- [ ] 5. tshark test of every encoded message, README, Modules page, log entry with shared page notes
- [ ] 6. Run the checks, open the PR, wait for CI, hand over for review

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Step 1: write modules/ldap/native/src/ber.rs.

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
| 2026-11-11T03:46Z | claude-e41a7f | claim | pending | Claimed issue #184 with a 90 minute lease, branch from origin/main |
