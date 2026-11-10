# Handoff CON-21 Scenario totals, criterion filters, and realism results in the REST report

| Field | Value |
|---|---|
| Issue | #390 |
| Branch | `wp/CON-21` |
| Pull request | none yet |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-c21a7f, lease until 2026-11-10T15:53:04Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add optional fields to the REST OpenAPI report, additive only: `RunReport.scenarios` (sessions, bytes, and failures per scenario and application), `CriterionResult.where` (the label filter of the criterion), and document where `RunReport.realism` comes from. The owner approved the change on 2026-11-10. CTL-38 runs in parallel and adds a scenarios table to the controller run report, so the field names must suit it. Stay inside `contracts/`.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read the report code, the UI-05 log entry, the realism README and the CTL-38 entry; decide the field names
- [x] 2. Edit `contracts/openapi/fps-api-v1.yaml` (0.9.0) and write `contracts/changes/CON-21.md`
- [x] 3. Contracts page, log entry, and the contract checks
- [ ] 4. Pull request, CI, and review

## Done and verified

Steps 1 to 3. `python3 tools/contracts/validate.py` reports 0 errors and 0 warnings, `python3 -m pytest -q tools/contracts` gives 290 passed and 1 skipped, and `plan.py check`, `render --check`, and `lint` pass. `generate_client.py --output <scratch>` accepts the new schemas. oasdiff cannot run here (no Go module proxy, and a downloaded binary was refused), so a script compared the base and the new document: paths, every other schema, and the old required lists and types are unchanged, and only properties and schemas were added. The openapi-breaking workflow is the real check.

## Remaining

The pull request and CI. Files outside `contracts/` that must change are listed in the change entry and in the pull request, and the coordinator applies them.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

The version is 0.9.0 (minor, as CON-12 to CON-20). `ScenarioTotals` uses the names of `RunSummary` and all counts are required, so CTL-39 sends zeros. `scenarios` stays optional for older controllers. `RealismResult` is a named schema with the optional `metric` and `labels`. The source of `realism` is the realism report of the run, and no operation delivers it to the controller, which the entry names as an open follow up.

## Pitfalls

A plain YAML scalar in a flow mapping or a block with `: ` fails to parse, so quote descriptions that hold commas or colons.

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest -q tools/contracts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T15:23Z | claude-c21a7f | claim | none | Claimed issue #390, branch wp/CON-21 |
| 2026-11-10T15:45Z | claude-c21a7f | checkpoint | see branch | OpenAPI 0.9.0, change entry, Contracts page, log entry, checks pass |
