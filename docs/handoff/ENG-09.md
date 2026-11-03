# Handoff ENG-09 Statistics collection

| Field | Value |
|---|---|
| Issue | #29 |
| Branch | `wp/ENG-09` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-eng-09-0e7e, lease until 2026-11-03T04:13:53Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Collect the catalog counters and HDR latency histograms per worker without locks and publish one snapshot per second. The crate `engine/fps-stats` provides per worker counter shards and histograms keyed by catalog names, and a snapshot type that merges shards and serializes to the proto3 JSON mapping of `CounterSnapshot`.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim, branch, handoff, pull request
- [ ] 2. Catalog table and its contract test against `contracts/metrics/catalog.yaml`
- [ ] 3. Worker shard with series registration, counters, gauges, and HDR histograms
- [ ] 4. Shard snapshots, merge, and the proto3 JSON mapping with its contract test
- [ ] 5. Concurrency and property tests, wiki section, log entry, finish

## Done and verified

None yet.

## Remaining

Everything in the plan.

## Next action

Write `src/catalog.rs` with the 48 metric definitions and a test that reads the YAML.

## Blockers and open questions

None.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
export CARGO_TARGET_DIR=/tmp/fps-target
cargo test -p fps-stats
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:45:00Z | claude-eng-09-0e7e | claim | none | Card 6594100381 |
