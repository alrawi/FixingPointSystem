# Handoff ENG-09 Statistics collection

| Field | Value |
|---|---|
| Issue | #29 |
| Branch | `wp/ENG-09` |
| Pull request | #87 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-eng-09-0e7e, lease until 2026-11-03T04:23:05Z |
| Last verified commit | see the newest session log row |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Collect the catalog counters and HDR latency histograms per worker without locks and publish one snapshot per second. The crate `engine/fps-stats` provides per worker counter shards and histograms keyed by catalog names, and a snapshot type that merges shards and serializes to the proto3 JSON mapping of `CounterSnapshot`.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim, branch, handoff, pull request
- [x] 2. Catalog table and its contract test against `contracts/metrics/catalog.yaml`
- [x] 3. Worker shard with series registration, counters, gauges, and HDR histograms
- [x] 4. Shard snapshots, merge, and the proto3 JSON mapping with its contract test
- [x] 5. Concurrency and property tests, wiki section, log entry, finish

## Done and verified

Each step is verified by `cargo test -p fps-stats`, which passes with 9 unit tests, 4 catalog contract tests, 4 JSON contract tests, the concurrency test, the property test, and the crate doc test.

1. Claim, branch, handoff, and pull request #87.
2. `src/catalog.rs` holds the 48 metrics, `tests/catalog.rs` compares them with the YAML file.
3. `src/shard.rs` holds the shard, series keys, and handles, with unit tests for registration, saturation, gauges, histograms, and foreign handles.
4. `src/snapshot.rs` holds the merge and the JSON mapping, `tests/proto_json.rs` compares the keys with `agent.proto` and decodes `hdrV2`.
5. `tests/concurrency.rs` and `tests/merge_props.rs`, the Engine page section, and the log entry. `make lint-rust lint-shell lint-deny test-eng` passes.

## Remaining

We added no performance test. The Testing page asks ENG for one, and ENG-10 or a later benchmark package can measure the shard hot path inside the worker runtime. The once per second publishing loop belongs to ENG-10, which owns the worker runtime.

## Next action

A reviewer reviews pull request #87.

## Blockers and open questions

None.

## Decisions taken in this package

1. Workers own their shards without atomics, and the worker hands a cumulative snapshot to the collector each interval. This keeps the hot path free of shared memory and makes a lost snapshot harmless.
2. Gauges merge by summing, so a shard rejects gauges in the units ratio, percent, and percentage_points. Those values are derived after the merge.
3. Histograms auto resize from a tiny start and clamp at one hour in microseconds, which keeps memory small for the many `actions.latency` series. Before encoding we copy each histogram into bounds set by its largest value, so the V2 header does not depend on the recording order.
4. The JSON follows the canonical proto3 printer and omits default values.
5. A series key may omit catalog labels, which aggregates over them, because test criteria name some metrics without labels.

## Pitfalls

The V2 encoding of an auto resizing histogram records its highest trackable value, so two histograms with equal counts can encode to different bytes. The property test found this, and its seed is kept in `tests/merge_props.proptest-regressions`.

## Verify the current state

```bash
cargo test -p fps-stats   # every test passes
make lint-rust lint-deny   # passes
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:45:00Z | claude-eng-09-0e7e | claim | none | Card 6594100381 |
| 2026-11-03T02:52:00Z | claude-eng-09-0e7e | checkpoint | see git log | Shards, snapshots, JSON mapping, tests |
| 2026-11-03T02:58:00Z | claude-eng-09-0e7e | finish | see git log | Wiki section and log entry, ready for review |
