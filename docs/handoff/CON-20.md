# Handoff CON-20 Stream events, metric source label, and provenance fields

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #377 |
| Branch | `wp/CON-20` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-13004c, lease until 2026-11-10T14:16:43Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Document the run SSE stream events (names, ids, retry, end event, 501) in the REST OpenAPI, add a source label to the mix metrics, and add optional provenance fields (capture_sha256, scrub_rules with versions, scrubbed_at). Additive only. Owner approved 2026-11-10.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Read UI-04 and controller stream code; document stream events and 501 in the REST OpenAPI
- [ ] 2. Add the source label to the mix metrics in the catalog; add provenance fields
- [ ] 3. Version bumps, contracts/changes/CON-20.md, log entry, Contracts.md, run checks
- [ ] 4. PR and CI

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
| 2026-11-10T12:50Z | claude-13004c | claim | | Claimed #377 |
