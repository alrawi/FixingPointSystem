# Handoff CON-05 Agent control protocol v0.2

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #154 |
| Branch | `wp/CON-05` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-con05a1, lease until 2026-11-06T22:36:24Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add UpdateRates, StartCapture, and StopCapture to the agent control protocol and signature fields to ModuleRef, with no breaking change against v0.1. CON-11 later adds `settings` to ModuleRef.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Proto messages and ModuleRef signature fields
- [ ] 2. buf format, lint, breaking, and checker tests
- [ ] 3. Change note and Contracts wiki page
- [ ] 4. Pull request and CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Edit agent.proto.

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
| 2026-11-06T22:07Z | claude-con05a1 | claim | | Claimed #154 |
