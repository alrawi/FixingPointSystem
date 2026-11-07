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
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-eng12a, lease until 2026-11-07T18:52:38Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Mirror a configurable, seeded sample of sessions into a bounded pcapng ring with TLS key logs, in the new crate `engine/fps-capture`. Acceptance: a 1% sample of 10,000 sessions yields between 70 and 130 captured sessions.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Sampler and pcapng writer
- [ ] 2. Frame synthesis (Ethernet, IPv4, IPv6, TCP, UDP)
- [ ] 3. Session tap and recorder
- [ ] 4. Ring with rotation and key logs
- [ ] 5. Tests, tshark check
- [ ] 6. Wiki, log entry, PR and CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Create the crate and write the sampler.

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
| 2026-11-07T17:23:00Z | claude-eng12a | claim | none | Claimed #157 |
