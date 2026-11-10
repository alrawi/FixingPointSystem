# Handoff CTL-39 Scenario totals and realism results in the REST report

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #391 |
| Branch | `wp/CTL-39` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-39c7a1, lease until 2026-11-10T18:55:02Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Report.API fills RunReport.scenarios from the CTL-38 scenario table and CriterionResult.where from the configuration, and carries stored realism results (empty until CON-22 and its follow up store them).

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Read report.go, scenarios.go, criteria.go, rest.go and the generated types
- [ ] 2. Map scenarios, where and stored realism in Report.API
- [ ] 3. Tests with literal values
- [ ] 4. Wiki ingest, log entry, checks, PR

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Read controller/internal/report/rest.go and scenarios.go.

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
| 2026-11-10T18:26Z | claude-39c7a1 | claim | - | Claimed #391, branch created from origin/main |
