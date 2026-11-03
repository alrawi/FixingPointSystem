# Handoff CON-08 Metric catalog views, reasons, and side label

| Field | Value |
|---|---|
| Issue | #133 |
| Branch | `wp/CON-08` |
| Pull request | #145 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-con-08-ec15, lease until 2026-11-03T20:35:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 13, 15, 17, and 21 to the metric catalog. Every metric names its API `MetricView`, the histogram statistic suffixes become a documented fixed list, the reasons `datagram_rejected`, `template_error`, and `extension_error` join the `reason` label, and session and transport metrics gain the `side` label.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Extend `contracts/metrics/catalog.schema.json` with `view` and `histogram_statistics`.
- [x] 2. Update `contracts/metrics/catalog.yaml` with views, reasons, side labels, and the suffix list.
- [x] 3. Keep the contract, engine, and controller tests green, with minimal mirror edits where CI fails.
- [x] 4. Add `contracts/changes/CON-08.md`, the Contracts page section, and the log entry.
- [ ] 5. Run the checks, push, wait for CI, and finish the pull request.

## Done and verified

1. `python3 tools/contracts/validate.py` reports 0 errors, and `python3 -m pytest -q tools/contracts` passes 153 tests. A missing `view`, a view that does not fit the level, and an extra histogram suffix each fail the schema.
2. `cargo test --workspace` passes with the engine mirrors updated, including `engine/fps-stats/tests/catalog.rs` and `engine/fps-interp/tests/catalog.rs`.
3. `go test ./...` in `controller` passes when `controller/internal/metrics/catalog.yaml` is replaced by the new catalog. We did not commit that copy.
4. `python3 -m pytest -q realism/tests` passes, and `tools/plan/plan.py check`, `render --check`, and `lint` pass.

## Remaining

The controller byte copy `controller/internal/metrics/catalog.yaml` must be refreshed by a CTL package, because `TestCatalogMatchesContract` compares it with the contract file. Engine code does not report the new reasons or the side label yet, which ENG-14 adds.

## Next action

Wait for the review of pull request #145.

## Blockers and open questions

The package carries `needs:owner`. The owner decided the content in D12, so the remaining owner step is the review of the catalog change.

## Decisions taken in this package

1. `view` is one value per metric. Scenario metrics name `scenario`, and the application view also serves them summed over `scenario`, because the API keys application samples by application.
2. Flow action metrics name `scenario`, because the API has no action view and their labels start with application and scenario.
3. The `side` label goes on every scenario level metric and every transport metric. It follows `scenario` or `port` in each label list, and the new reasons follow `internal`, so earlier positions stay stable.
4. A total over both sides counts each session twice, so the catalog states that API views and criteria report the client side by default. This matches the agent, which already reports the client side.
5. The schema pins `histogram_statistics` with `const` and checks that each view fits its level.
6. The catalog version moves to 0.2.0, because catalog 0.1.0 shipped in contracts 0.1.1.
7. We edited the engine mirror tables and `Reason`, and one test fixture in `tools/contracts/test_contracts.py`, because `ci.yml` runs `cargo test --workspace` and the contract tests on every pull request and they fail without these edits.

## Pitfalls

The controller workflow does not run on `contracts/metrics/**`, so the controller copy test stays silent in this pull request and fails on the next controller pull request.

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
