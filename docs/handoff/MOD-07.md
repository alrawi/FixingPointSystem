# Handoff MOD-07 SMTP module

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #174 |
| Branch | `wp/MOD-07` |
| Pull request | none |
| Status | in progress |
| Progress | 0% (0 of 8 plan steps) |
| Current holder | claude-mod07-a1, lease until 2026-11-09T17:42:06Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship smtp client and server scenarios (EHLO, AUTH PLAIN and LOGIN, MAIL, RCPT, DATA with a MIME attachment, QUIT) with a submission variant. Zeek logs every fixture message without parser errors.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Read the engine, http1, dns, content, and tls modules and settle the STARTTLS question
- [ ] 2. Native crate with smtp functions and unit tests
- [ ] 3. Manifest and scenarios, validate.py passes
- [ ] 4. Interpreter tests with an SMTP server model
- [ ] 5. Golden transcripts at seed 42
- [ ] 6. Fixture capture script and expected.yaml
- [ ] 7. mk and workflow notes, README, wiki section, log entry
- [ ] 8. Checks, pull request, CI

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
| 2026-11-09T16:12Z | claude-mod07-a1 | claim | none | Claimed issue 174 |
