# Handoff CON-28 Packet gap and payload size series in the metric catalog

| Field | Value |
|---|---|
| Issue | #496 |
| Branch | `wp/CON-28` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-4d21ba, lease until 2026-11-12T15:15:55Z |
| Last verified commit | none |

## Goal

The property label of `realism.ks_statistic` and `realism.ks_p_value` gets values for packet gaps (catalog 0.8.0), and the realism runner emits those series, so packet gaps reach `RunReport.realism`. The owner approved the contract change on 2026-11-12 (comment on #496). The coordinator authorized the consumer follow ups outside `contracts/` in this PR.

## Plan

- [ ] 1. Contract: catalog 0.8.0, examples, `contracts/changes/CON-28.md`, changelog
- [ ] 2. Consumers: controller catalog copy, version pins, `catalog.rs`
- [ ] 3. Realism runner: series, schema, report format, tests, controller schema copy, `schema.d.ts`
- [ ] 4. Docs: Realism page, README, log entry
- [ ] 5. Checks, PR, CI, review

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Edit `contracts/metrics/catalog.yaml`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 tools/contracts/validate.py
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T14:46Z | claude-4d21ba | claim | none | claimed #496 |
