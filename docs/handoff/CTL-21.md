# Handoff CTL-21 Throughput rate calibration

| Field | Value |
|---|---|
| Issue | #153 |
| Branch | `wp/CTL-21` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-953995, lease until 2026-11-06T23:01:51Z |
| Last verified commit | none |

## Goal

Apply decision D12 item 10 in the new package `controller/internal/runs/calibrate/`. Given a throughput objective, an initial bytes per session estimate from the manifest field `expected_bytes`, and periodic measurements of bytes and sessions, the package returns a corrected session rate factor with bounded steps. It stays pure and deterministic and is not wired into the orchestrator.

## Plan

- [ ] 1. Package with configuration, state, and the bounded update step.
- [ ] 2. Convergence tests with 0.25x and 4x estimates and noisy measurements.
- [ ] 3. Make targets in `mk/ctl.mk`.
- [ ] 4. Controller wiki page, log entry, and pull request.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write `controller/internal/runs/calibrate/calibrate.go`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cd controller && GOWORK=off go test -race ./internal/runs/calibrate/
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T21:32Z | claude-953995 | claim | none | Lease until 23:01:51Z |
