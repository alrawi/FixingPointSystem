# Handoff CTL-21 Throughput rate calibration

| Field | Value |
|---|---|
| Issue | #153 |
| Branch | `wp/CTL-21` |
| Pull request | #223 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-953995, lease until 2026-11-06T23:01:51Z |
| Last verified commit | bdc8b85 |

## Goal

Apply decision D12 item 10 in the new package `controller/internal/runs/calibrate/`. Given a throughput objective, an initial bytes per session estimate from the manifest field `expected_bytes`, and periodic measurements of bytes and sessions, the package returns a corrected session rate factor with bounded steps. It stays pure and deterministic and is not wired into the orchestrator.

## Plan

- [x] 1. Package with configuration, state, and the bounded update step.
- [x] 2. Convergence tests with 0.25x and 4x estimates and noisy measurements.
- [x] 3. Make targets in `mk/ctl.mk`.
- [x] 4. Controller wiki page, log entry, and pull request.

## Done and verified

- `cd controller && GOWORK=off go test -race -cover ./internal/runs/calibrate/` passes with 100 percent statement coverage, and `golangci-lint run ./internal/runs/calibrate/` reports 0 issues. The module proxy is unreachable, but the package uses only the standard library, so it builds and tests locally.
- `make lint-ctl-calibrate test-ctl-calibrate`, `plan.py check`, `render --check`, and `lint` pass.

## Remaining

CI is green on bdc8b85. The orchestrator wiring is out of scope and needs CON-05 UpdateRates.

## Next action

Review PR #223.

## Blockers and open questions

none

## Decisions taken in this package

- The calibration estimates bytes per session from completed sessions and does not feed back the achieved throughput, so it has no feedback delay and cannot oscillate with agent ramp up.
- One common Factor scales all compiled rates, because the compiler splits a single total rate over the mix by session weight.
- Moves against the last direction need a 1.5x wider dead band, which stops jitter around a settled factor.

## Pitfalls

The shell tool refuses `source` and long heredocs in worktrees, so pass FPS_* variables inline and write files with the editor tools.

## Verify the current state

```bash
cd controller && GOWORK=off go test -race ./internal/runs/calibrate/
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T21:32Z | claude-953995 | claim | none | Lease until 23:01:51Z |
