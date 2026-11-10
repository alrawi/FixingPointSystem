# Handoff MOD-12 SMB2 encoding functions

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #179 |
| Branch | `wp/MOD-12` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-281119, lease until 2026-11-10T15:53:05Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement SMB2 message encoding and parsing for negotiate, session setup, tree connect, create, read, write, and close as a native Rust crate `fps-module-smb2` under `modules/smb2/native/`, with synthetic NTLM exchanges. Acceptance: tshark parses every encoded message without errors.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Crate skeleton, args, catalog
- [ ] 2. SMB2 header, negotiate, close, and other encoders
- [ ] 3. Synthetic NTLM (NEGOTIATE, CHALLENGE, AUTHENTICATE) and session setup
- [ ] 4. Readers and golden bytes tests
- [ ] 5. Fuzz, catalog test, tshark script
- [ ] 6. Log entry, Modules page, checks, PR, CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Step 1: create the crate.

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
| 2026-11-10 | claude-281119 | claim | | Claimed #179 |
