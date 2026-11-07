# Handoff ENG-13 Run time rate adjustment

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #158 |
| Branch | `wp/ENG-13` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-7a2195, lease until 2026-11-07T18:53:08Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Let the scheduler of `engine/fps-sched/` change per scenario rates during a run without restarting workers. A thread safe handle takes effect at the next scheduling tick, the arrival process stays correct across the change, and rate 0 pauses.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Rate handle and rebase of the arrival process in `Schedule`
- [ ] 2. `RateControl` registry with all or nothing updates
- [ ] 3. Tests (tick, realized rate on a simulated clock, pause, unknown id, threads)
- [ ] 4. Wiki page, log entry, checks, pull request

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
| 2026-11-07 | claude-7a2195 | claim | none | Card 6710609923 |
