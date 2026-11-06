# Handoff CTL-20 Run reasons, criterion results, and criterion filters

| Field | Value |
|---|---|
| Issue | #137 |
| Branch | `wp/CTL-20` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-95e128, lease until 2026-11-06T16:49:52Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 14 and 19 and close note 4 of the CTL-09 log entry: fill `Run.reason` for every failed run, return `observed`, `passed`, and `reason` for every criterion, apply the `where` filter of a criterion to labeled metrics (with the client side default of CON-08), and carry the run id in `runs.Outcome`.

## Plan

- [ ] 1. `runs`: reason text, `Outcome.RunID`, labeled counters, criterion type with `where`, `ToAPI` fields
- [ ] 2. `report`: label aware criteria evaluation with the client side default
- [ ] 3. `report`: run evaluator that reads the aggregator by run id
- [ ] 4. Tests and golden reports
- [ ] 5. Docs, log entry, lint, and CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Start step 1.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

Go modules cannot be downloaded in the sandbox, so CI is the build and test loop.

## Verify the current state

```bash
make lint-ctl test-ctl
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T16:20Z | claude-95e128 | claim | none | Fresh claim from origin/main |
