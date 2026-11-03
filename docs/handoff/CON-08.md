# Handoff CON-08 Metric catalog views, reasons, and side label

| Field | Value |
|---|---|
| Issue | #133 |
| Branch | `wp/CON-08` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-con-08-ec15, lease until 2026-11-03T20:35:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 13, 15, 17, and 21 to the metric catalog. Every metric names its API `MetricView`, the histogram statistic suffixes become a documented fixed list, the reasons `datagram_rejected`, `template_error`, and `extension_error` join the `reason` label, and session and transport metrics gain the `side` label.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Extend `contracts/metrics/catalog.schema.json` with `view` and `histogram_statistics`.
- [ ] 2. Update `contracts/metrics/catalog.yaml` with views, reasons, side labels, and the suffix list.
- [ ] 3. Keep the contract, engine, and controller tests green, with minimal mirror edits where CI fails.
- [ ] 4. Add `contracts/changes/CON-08.md`, the Contracts page section, and the log entry.
- [ ] 5. Run the checks, push, wait for CI, and finish the pull request.

## Done and verified

None yet.

## Remaining

Every plan step.

## Next action

Extend the catalog schema with the `view` field.

## Blockers and open questions

The package carries `needs:owner`.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest -q tools/contracts
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T19:05Z | claude-con-08-ec15 | claim | none | card 6612946675 |
