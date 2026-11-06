# Handoff ENG-14 Failure reasons and side label in engine statistics

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #139 |
| Branch | `wp/ENG-14` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-68b23a, lease until 2026-11-06T16:47:46Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply the CON-08 catalog additions in the engine statistics, so failure reasons (datagram_rejected, template_error, extension_error) and the client or server side reach the snapshots. Only engine/fps-stats/ is in scope.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Typed Side and FailureReason in fps-stats, with tests against the catalog
- [ ] 2. Session and transport counter sets with the side label, snapshot tests
- [ ] 3. Engine wiki page, log entry, findings for callers outside fps-stats
- [ ] 4. Checks and pull request

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
| 2026-11-06 | claude-68b23a | claim | none | Claimed #139 |
