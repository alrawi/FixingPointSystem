# Handoff CTL-39 Scenario totals and realism results in the REST report

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #391 |
| Branch | `wp/CTL-39` |
| Pull request | #411 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-39c7a1, lease until 2026-11-10T18:55:02Z |
| Last verified commit | dab9d57 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Report.API fills RunReport.scenarios from the CTL-38 scenario table and CriterionResult.where from the configuration, and carries stored realism results (empty until CON-22 and its follow up store them).

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read report.go, scenarios.go, criteria.go, rest.go and the generated types
- [x] 2. Map scenarios, where and stored realism in Report.API
- [x] 3. Tests with literal values
- [x] 4. Wiki ingest, log entry, checks, PR

## Done and verified

Steps 1 to 3 are written. `gofmt -l controller/internal/report` prints nothing. Go cannot build here, so CI is the build and test loop.

## Remaining

Nothing. CI is green at dab9d57.

## Next action

Review PR 411.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- `scenarioRows` is the one pass over the series; `scenarioTable` (CTL-38 file form) and `scenarioTotals` (REST form) derive from it, so ScenarioCount and its tests stay as they were.
- `Report.ScenarioTotals` holds the REST form, `Report.Realism` the stored realism results; `API()` always sends both lists, empty when there are none (contract 0.9.0).
- The file report keeps the CTL-38 table: the outer `Scenarios` field of `fileReport` shadows the embedded REST property in encoding/json.
- `where` is copied in `Report.API` because `runs.ToAPI` is outside the package paths.

## Pitfalls

The goldens `pass`, `fail`, `degraded` end with `"scenarios": []`; `labeled.json` holds the literal table and `where` on seven criteria. Controller.md had a heading merged with a sentence (CTL-38); repaired.

## Verify the current state

```bash
gofmt -l controller/internal/report   # prints nothing
# CI: go test -race ./internal/report/...
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T18:26Z | claude-39c7a1 | claim | - | Claimed #391, branch created from origin/main |
| 2026-11-10T18:50Z | claude-39c7a1 | finish | dab9d57 | PR 411 opened, CI green (ctl, ci, cli-go, agt, qa-gate1, ops, containers, image) |
