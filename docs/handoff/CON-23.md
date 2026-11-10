# Handoff CON-23 Catalog metrics for every realism check

| Field | Value |
|---|---|
| Issue | #430 |
| Branch | `wp/CON-23` |
| Pull request | #434 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-0540fd, lease until 2026-11-11T00:05:13Z |
| Last verified commit | 38db05f |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Every check kind of `realism/schema/realism-report.schema.json` reports at least one catalog metric whose value and target say whether it passed. The metric catalog gains additive metrics (and labels where needed), and `contracts/changes/CON-23.md` extends the CON-22 mapping rule from report metrics to `RealismResult` entries. The owner chose catalog metrics per check kind on 2026-11-10. The change stays inside `contracts/`. The report schema change and every file outside `contracts/` go into the report as exact follow-up changes.

## Plan

- [x] 1. Read what each realism check computes and design the metrics.
- [x] 2. Edit the catalog (version bump), the examples, and write `contracts/changes/CON-23.md`.
- [x] 3. Run `validate.py`, the contract tests, and the plan checks.
- [x] 4. Open the PR, wait for CI, and request review.

## Done and verified

- Catalog 0.5.0 (seven gauges, labels `check` and `parser`, `property` values), OpenAPI description text, `contracts/changes/CON-23.md`, the schema patch, `contracts/examples/realism/check-metrics.yaml`, Contracts page, and log entry are written.
- `python3 tools/contracts/validate.py`: 0 errors. `python3 -m pytest -q tools/contracts`: 288 passed, 2 failed as expected (version pin 0.4.0 and the byte copy of the catalog in `controller/`), until the coordinator applies the follow up.
- `plan.py check`, `render --check`, and `lint`: ok.
- Throwaway scripts checked every example case against the catalog and against the patched schema (the patch applies with `patch -p1` and gives the schema that the cases were tested with), including rejections.

## Remaining

The follow up outside `contracts/` is for the coordinator: the byte copy of the catalog, two version pins, the engine table, and three regenerated files. CI is red on exactly those (see the PR comment).

## Next action

Coordinator: apply the follow up list of `contracts/changes/CON-23.md` on the branch.

## Blockers and open questions

none

## Decisions taken in this package

- A uniform `realism.check_failures` series for every check, because several checks fail with no per thing metric (no parser ran, no TLS session sampled, no profiled handshake, run counters off in the mix). Its value is 0 exactly when the check passed.
- New metrics sit at the end of the catalog so the engine metric identifiers (catalog positions) do not move.
- The OpenAPI version stays 0.10.0: only descriptions change (precedent CON-16).
- The schema change is a patch file in `contracts/changes/`, because the schema is outside this package.

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
| 2026-11-10 | claude-0540fd | checkpoint | see branch | Catalog, change entry, examples, patch, and wiki written; contract checks green except the two expected pins |
| 2026-11-10 | claude-0540fd | finish | 38db05f | PR 434 opened, CI red only on the listed follow up files, review requested |
