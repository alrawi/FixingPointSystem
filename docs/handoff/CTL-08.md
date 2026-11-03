# Handoff CTL-08 Metrics aggregation and streaming

| Field | Value |
|---|---|
| Issue | #46 |
| Branch | `wp/CTL-08` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-ctl-08-3dfe, lease until 2026-11-03T15:51:23Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Merge the cumulative agent counter snapshots of a run into run metrics, serve them at `GET /v1/runs/{id}/metrics` with views and windows, stream them over Server Sent Events at `GET /v1/runs/{id}/metrics:stream`, and expose a Prometheus `/metrics` endpoint. Acceptance requires that aggregated counters equal the sum of fake agent snapshots and that histogram merges match a reference merge.

## Plan

- [ ] 1. Read contracts, fps-stats snapshot format, CTL-06 observer, and CTL-07 runs
- [ ] 2. Snapshot decoding and the aggregator with counter, gauge, and histogram merges
- [ ] 3. The metrics view handler with views and windows
- [ ] 4. The SSE stream handler
- [ ] 5. The Prometheus collector and handler
- [ ] 6. Docs, lint, tests, and finish

## Done and verified

None yet.

## Remaining

Everything.

## Next action

Read the contracts and the agent manager observer.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cd controller && GOWORK=off go test ./internal/metrics/...
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T14:21Z | claude-ctl-08-3dfe | claim | none | Claimed with card 6606272183 |
