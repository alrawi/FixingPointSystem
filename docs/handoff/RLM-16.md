# Handoff RLM-16 Deliver the realism report to the controller

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #426 |
| Branch | `wp/RLM-16` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-492e74, lease until 2026-11-12T15:15:59Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A `score` option that PUTs the report to `PUT /v1/runs/{id}/realism` (attachRealismReport) with a run token read from a file or the environment, never the command line, and reports the controller's answer. Acceptance: a test against a local fake controller checks the request, the token handling, and the 409 and 422 answers.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Delivery module realism/deliver.py with token, URL, PUT, answers
- [ ] 2. CLI options and tests against a fake controller
- [ ] 3. README and Realism page, log entry, checks, PR

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Write realism/deliver.py.

## Blockers and open questions

none

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
| 2026-11-12 | claude-492e74 | claim | | Claimed #426 |
