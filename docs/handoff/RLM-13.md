# Handoff RLM-13 Mix accuracy from the classifier as metric series

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #392 |
| Branch | `wp/RLM-13` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-d73749, lease until 2026-11-10T18:55:18Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The mix accuracy check emits mix.share_pct, mix.abs_error_pp and mix.max_abs_error_pp with source=classifier and the classifier label (catalog 0.4.0), and reads the scenarios table of the REST report beside the file report form.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Metric series in the mix entry, schema, report version 0.10.0
- [ ] 2. REST report scenarios table input
- [ ] 3. Tests on a fixture with a known mix, catalog 0.4.0 validation
- [ ] 4. README, Realism page, log entry, PR

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
| 2026-11-10T18:15Z | claude-d73749 | claim | - | claimed #392 |
