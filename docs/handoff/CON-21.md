# Handoff CON-21 Scenario totals, criterion filters, and realism results in the REST report

| Field | Value |
|---|---|
| Issue | #390 |
| Branch | `wp/CON-21` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-c21a7f, lease until 2026-11-10T15:53:04Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add optional fields to the REST OpenAPI report, additive only: `RunReport.scenarios` (sessions, bytes, and failures per scenario and application), `CriterionResult.where` (the label filter of the criterion), and document where `RunReport.realism` comes from. The owner approved the change on 2026-11-10. CTL-38 runs in parallel and adds a scenarios table to the controller run report, so the field names must suit it. Stay inside `contracts/`.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Read the report code, the UI-05 log entry, the realism README and the CTL-38 entry; decide the field names
- [ ] 2. Edit `contracts/openapi/fps-api-v1.yaml` (0.9.0) and write `contracts/changes/CON-21.md`
- [ ] 3. Contracts page, log entry, and the contract checks
- [ ] 4. Pull request, CI, and review

## Done and verified

Nothing yet.

## Remaining

Everything. Files outside `contracts/` that must change are listed in the change entry and in the pull request, and the coordinator applies them.

## Next action

Edit the OpenAPI document.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest -q tools/contracts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T15:23Z | claude-c21a7f | claim | none | Claimed issue #390, branch wp/CON-21 |
