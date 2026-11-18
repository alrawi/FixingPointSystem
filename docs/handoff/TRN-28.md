# Handoff TRN-28 OS profile options for the own stack

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #629 |
| Branch | `wp/TRN-28` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-7a7cf0, lease until 2026-11-18T12:59:34Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Map the OS profile fields of the test configuration (TTL, window, MSS, window scale, and option order) onto the SYN and SYN-ACK of the own stack, as the kernel backend does through socket options today. The acceptance: the SYN and SYN-ACK of the own stack match the option order and values of every OS profile in Transport.md.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Read the own stack handshake code and the kernel profile table
- [ ] 2. Profile type and the mapping onto the SYN and SYN-ACK
- [ ] 3. Tests that compare the SYN and SYN-ACK bytes of each profile with the table
- [ ] 4. Wiki ingest (log entry, Transport page), checks, pull request

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
| 2026-11-18T11:30Z | claude-7a7cf0 | claim | | Claimed #629 with 4 steps |
