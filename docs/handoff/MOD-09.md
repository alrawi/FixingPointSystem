# Handoff MOD-09 POP3 module

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #176 |
| Branch | `wp/MOD-09` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-406366, lease until 2026-11-09T21:23:56Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship the pop3 module under modules/pop3/: scenarios for USER and PASS login and APOP, STAT, LIST, UIDL, RETR, TOP, DELE, RSET, QUIT. nDPI must classify at least 99% of fixture sessions as POP3.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Native crate and manifest
- [ ] 2. Scenarios, tests, golden transcripts
- [ ] 3. Fixture, capture.sh, mk and workflow
- [ ] 4. Docs, log entry, PR and CI

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
