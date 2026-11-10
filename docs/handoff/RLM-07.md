# Handoff RLM-07 Mix accuracy check

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #189 |
| Branch | `wp/RLM-07` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-7a1c07, lease until 2026-11-10T06:15:51Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Compute the achieved application mix from run metrics and from nDPI output, and compare both with the configured weights for the session and byte bases within a tolerance in percentage points (default 2). A report that is off by 3 points for one application fails.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Labels `mix` section and the metrics/config readers
- [ ] 2. `realism/mix.py` check, report entry, schema 0.8.0, CLI
- [ ] 3. Tests with synthetic run reports and pcapgen captures
- [ ] 4. Docs, log entry, checks, PR

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Step 1.

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
