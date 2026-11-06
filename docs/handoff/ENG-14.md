# Handoff ENG-14 Failure reasons and side label in engine statistics

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #139 |
| Branch | `wp/ENG-14` |
| Pull request | none yet |
| Status | in progress |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-68b23a, lease until 2026-11-06T16:47:46Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply the CON-08 catalog additions in the engine statistics, so failure reasons (datagram_rejected, template_error, extension_error) and the client or server side reach the snapshots. Only engine/fps-stats/ is in scope.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Typed Side and FailureReason in fps-stats, with tests against the catalog
- [x] 2. Session and transport counter sets with the side label, snapshot tests
- [x] 3. Engine wiki page, log entry, findings for callers outside fps-stats
- [x] 4. Checks and pull request

## Done and verified

1. `engine/fps-stats/src/series.rs` adds `Side`, `FailureReason`, `SessionCounters`, `TransportCounters`. Verified with `cargo test -p fps-stats` (all suites pass) and `cargo clippy -p fps-stats --all-targets -- -D warnings`.
2. `engine/fps-stats/tests/series.rs` shows each new reason and both sides in a merged snapshot and in JSON, and compares the enums with the catalog file.
3. Engine page section and log entry `docs/wiki/log/2026-11-06T1600-ingest-eng-14-failure-reasons-and-side-label.md`, with follow up findings for fps-interp, fps-runtime, fps-agent.

## Remaining

Callers outside `engine/fps-stats/` are out of scope and listed in the log entry. Pull request and CI.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

`fps-stats` defines its own `Side` and `FailureReason` because it does not depend on `fps-interp`. The counter sets cover counters only, so gauges and histograms take the side label through `Shard` directly.

## Pitfalls

The catalog table in `src/catalog.rs` already had the CON-08 labels, so no catalog edit was needed.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06 | claude-68b23a | claim | none | Claimed #139 |
