# Handoff CON-25 The messages check in the metric catalog

| Field | Value |
|---|---|
| Issue | #458 |
| Branch | `wp/CON-25` |
| Pull request | pending |
| Status | in progress |
| Progress | 80% (2 of 3 plan steps) |
| Current holder | claude-c25a7e, lease until 2026-11-11T19:41:41Z |
| Last verified commit | none |

## Goal

Add the value `messages` to the catalog label `check` (catalog 0.6.0), define `realism.check_failures` for it as `failures_total`, and add a passing and a failing case to `contracts/examples/realism/check-metrics.yaml`. The owner approved the contract change on 2026-11-11.

## Plan

- [x] 1. Catalog 0.6.0, examples, change entry `contracts/changes/CON-25.md`
- [x] 2. Contracts page, log entry, handoff
- [ ] 3. PR and CI

## Done and verified

- `python3 tools/contracts/validate.py`: 0 errors, 0 warnings.
- `pytest tools/contracts`: 288 passed, 2 failed (version pin and controller byte copy), both named in the follow up.

## Remaining

PR, CI, review hand-off. The follow up outside `contracts/` is the coordinator's.

## Next action

Open the PR and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- `messages` is appended to the end of the `check` values, so earlier values keep their place.
- No new mapping row: the generic `realism.check_failures` rule of CON-23 covers it.
- Files outside `contracts/` (controller copy, version pins, `catalog.rs`, `realism/`) are left to the coordinator, as for CON-23.

## Pitfalls

none

## Verify the current state

```bash
python3 tools/contracts/validate.py   # 0 errors
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-11 | claude-c25a7e | claim | none | claimed #458 |
