# Handoff CON-27 The host identity check in the metric catalog

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #480 |
| Branch | `wp/CON-27` |
| Pull request | #485 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-a2ef7e, lease until 2026-11-12T04:26:11Z |
| Last verified commit | 661e4f4 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the value `host_identity` to the label `check` of the metric catalog (0.7.0), define its `realism.check_failures` as `failures_total`, add a passing and a failing case to `contracts/examples/realism/check-metrics.yaml`, and state the rule for two checks of one kind. The owner approved it (comment on #480). The coordinator authorized the consumer follow ups outside `contracts/` in the same PR.

## Plan

- [x] 1. Contract: catalog 0.7.0, examples, `contracts/changes/CON-27.md`, Contracts page
- [x] 2. Consumer follow ups: controller copies, version pins, `catalog.rs`, runner (format 0.15.0), con23 count test, UI schema, docs
- [x] 3. Merge origin/main (RLM-15, CTL-46 may take format numbers), push, open the PR
- [x] 4. CI green, then `review`

## Done and verified

- `python3 tools/contracts/validate.py`: 0 errors. `pytest tools/contracts`: 290 passed.
- `pytest realism` (full): see the PR description. `test_check_metrics.py` and `test_hosts.py`: 102 passed.
- `plan.py check`, `render --check`, `lint`: ok.
- `npm ci` and `npm run gen:api` in `ui/`: only `schema.d.ts` changed.

## Remaining

Open the PR and watch CI. Go cannot build here, so the example counts (10 kinds, 21 cases) in `realism_con23_test.go` are checked by CI only.

## Next action

Merge `origin/main`, take the next free report format number if 0.15.0 is taken, push, open the PR.

## Blockers and open questions

none

## Decisions taken in this package

- Two checks of one kind: the document is invalid, with no combination of outcomes. The controller already reports the repeated metric and labels pair, so no controller code changes. CTL-42 may drop the `passedBy` AND in `MapRealism`, which cannot matter.
- Report format 0.15.0 (the next after 0.14.0).

## Pitfalls

- The UI schema is generated from the runner schema descriptions, so a schema text change needs `npm run gen:api`.

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest realism tools/contracts -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12 | claude-a2ef7e | claim | | claimed #480 |
| 2026-11-12 | claude-a2ef7e | checkpoint | | contract and follow ups written |
| 2026-11-12 | claude-a2ef7e | finish | 661e4f4 | PR #485, CI green (62 checks) |
