# Handoff CTL-08 Metrics aggregation and streaming

| Field | Value |
|---|---|
| Issue | #46 |
| Branch | `wp/CTL-08` |
| Pull request | #112 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-ctl-08-3dfe, lease until 2026-11-03T16:17:52Z |
| Last verified commit | d261bdc |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Merge the cumulative agent counter snapshots of a run into run metrics, serve them at `GET /v1/runs/{id}/metrics` with views and windows, stream them over Server Sent Events at `GET /v1/runs/{id}/metrics:stream`, and expose a Prometheus `/metrics` endpoint. Acceptance requires that aggregated counters equal the sum of fake agent snapshots and that histogram merges match a reference merge.

## Plan

- [x] 1. Read contracts, fps-stats snapshot format, CTL-06 observer, and CTL-07 runs
- [x] 2. Snapshot decoding and the aggregator with counter, gauge, and histogram merges
- [x] 3. The metrics view handler with views and windows
- [x] 4. The SSE stream handler
- [x] 5. The Prometheus collector and handler
- [x] 6. Docs, lint, tests, and finish

## Done and verified

Every step is verified by `make lint-ctl test-ctl` with `GOWORK=off`, which passed with 0 lint issues and every controller package green. The package coverage is 92.7%. A 20 second run of `FuzzDecodeHist` found no failure.

| Acceptance | Test |
|---|---|
| Aggregated counters equal the sum of fake agent snapshots | `TestCountersEqualSumOfFakeAgentSnapshots` with three fake agents through the real agent manager |
| Histogram merges match a reference merge | `TestMergeMatchesReference` and `TestHistogramsOfFakeAgentsMatchReference` against `testdata/reference_merge.json` from `Snapshot::merge_all` of fps-stats |

## Remaining

Three gaps stay outside the package paths.

1. The final snapshot of a `StopResult` reaches only the orchestrator, so the aggregator misses at most the last interval of an agent stopped by command. A hook in `internal/runs` can call `Aggregator.Add`.
2. The OpenAPI document does not list `/metrics`, and the generated `MetricSample` holds float32 values, so the package uses its own `Sample` type with float64 values and validates it against the schema in tests.
3. The `mix.` and `realism.` gauges pass through when an agent or pipeline sends them. No package computes them yet.

## Next action

A reviewer checks pull request #112 against the Definition of Done.

## Blockers and open questions

none

## Decisions taken in this package

- We decode HdrHistogram V2 ourselves in `hdr.go` and add no Go dependency. `hdrhistogram-go` panics on short input, allocates the dense counts array of the header, and cannot bound inflation.
- Every histogram merges in the fps-stats layout with lowest value 1 and 3 significant digits, so agent merges are exact. Other layouts re-bucket by bucket value.
- The Prometheus text format is written with the standard library, so `controller/go.mod` is unchanged and cannot conflict with CTL-17.
- Each source keeps the newest value per series, not the whole newest snapshot, because the fake agent and the orchestrator treat a final snapshot without counters as cumulative.
- Per worker snapshots of an agent replace its aggregate snapshot. Ratio gauges average, other gauges sum, and formula gauges are recomputed per group.
- Every view carries the key `*` for the whole run, and the scenario view keys samples as `application/scenario`.

## Pitfalls

- gosec G115 flags every integer conversion in tests too, and `.golangci.yml` is outside the package, so tests loop over `uint64` counters.
- `testdata/refgen` is a standalone Cargo package with its own `[workspace]`. Delete its `Cargo.lock` after a run.

## Verify the current state

```bash
cd controller && GOWORK=off go test -race ./internal/metrics/...   # ok
cd controller && GOWORK=off golangci-lint run ./internal/metrics/... # 0 issues
cargo run --release --manifest-path controller/internal/metrics/testdata/refgen/Cargo.toml | cmp - controller/internal/metrics/testdata/reference_merge.json
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T14:21Z | claude-ctl-08-3dfe | claim | none | Claimed with card 6606272183 |
| 2026-11-03T14:39Z | claude-ctl-08-3dfe | checkpoint | 4c8123b | Aggregator, decoder, handlers, reference merge test |
| 2026-11-03T14:47Z | claude-ctl-08-3dfe | checkpoint | d261bdc | Views, windows, stream, Prometheus tests |
| 2026-11-03T14:52Z | claude-ctl-08-3dfe | finish | see branch | Docs and handoff, ready for review |
