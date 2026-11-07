# Handoff CTL-23 Client side default in metric views, run seeds, and test stability

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #219 |
| Branch | `wp/CTL-23` |
| Pull request | none yet |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-c23a91, lease until 2026-11-07T19:14:28Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Client side default in metric views, the run seed from the compiled plans, two flaky tests fixed at their cause, and the workflow trigger for contracts/metrics/**.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Side default in metrics views with tests
- [x] 2. Seed from plans, race fixes for the two tests
- [x] 3. Workflow trigger, wiki page, log entry, temporary stress step
- [ ] 4. CI green, remove stress step, review

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Read the CI result of the stress step, remove it from controller.yml, push, and run review.

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
| 2026-11-07 | claude-c23a91 | claim | | claimed after lease release |
