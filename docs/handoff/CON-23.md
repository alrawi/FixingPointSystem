# Handoff CON-23 Catalog metrics for every realism check

| Field | Value |
|---|---|
| Issue | #430 |
| Branch | `wp/CON-23` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-0540fd, lease until 2026-11-11T00:05:13Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Every check kind of `realism/schema/realism-report.schema.json` reports at least one catalog metric whose value and target say whether it passed. The metric catalog gains additive metrics (and labels where needed), and `contracts/changes/CON-23.md` extends the CON-22 mapping rule from report metrics to `RealismResult` entries. The owner chose catalog metrics per check kind on 2026-11-10. The change stays inside `contracts/`. The report schema change and every file outside `contracts/` go into the report as exact follow-up changes.

## Plan

- [ ] 1. Read what each realism check computes and design the metrics.
- [ ] 2. Edit the catalog (version bump), the examples, and write `contracts/changes/CON-23.md`.
- [ ] 3. Run `validate.py`, the contract tests, and the plan checks.
- [ ] 4. Open the PR, wait for CI, and request review.

## Done and verified

Nothing yet.

## Remaining

All steps.

## Next action

Read `realism/parser_errors.py`, `encryption.py`, `content.py`, `structure.py`, `transport.py`, and `mix.py` for the values each check computes.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

The sandbox worktree guard refuses compound shell commands that mix `git` with other constructs.

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest -q tools/contracts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10 | claude-0540fd | claim | none | Claimed issue 430, branch wp/CON-23 from origin/main |
