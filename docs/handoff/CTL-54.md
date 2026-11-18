# Handoff CTL-54 The orchestrator inventory uses the selected backend

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #634 |
| Branch | `wp/CTL-54` |
| Pull request | #636 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-c54a7e, lease until 2026-11-18T02:07:42Z |
| Last verified commit | eb4d0fe |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The orchestrator inventory in controller/internal/wiring takes the selected backend of each agent instead of its first backend (CTL-53 finding), with the first known backend as the fallback for an old agent.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Inventory change and tests
- [x] 2. CI green on the PR
- [x] 3. Review handoff

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Write exactly one concrete step that the next agent can start immediately.

## Blockers and open questions

none

## Decisions taken in this package

Reuse the exported `agents.ToAPI(ag).Backend` so the REST view and the compiler share one rule.

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
| 2026-11-18 | claude-c54a7e | claim and step 1 | pending | Go cannot build locally; CI is the build loop |
| 2026-11-18 | claude-c54a7e | finish | eb4d0fe | CI green on #636 after re-run of the rust flake (bug #637) |
