# Handoff CTL-24 Throughput calibration in the run loop

| Field | Value |
|---|---|
| Issue | #232 |
| Branch | `wp/CTL-24` |
| Pull request | #269 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-6555a2, lease until 2026-11-09T05:23:25Z |
| Last verified commit | 9c9d3dc |

## Goal

For a `throughput_bps` objective, the orchestrator builds a `calibrate.State` from the compiled mix and `expected_bytes`, feeds it the client side bytes and completed sessions per mix entry every measurement window, and sends `UpdateRates` (CON-05) to agents with `engine_api` 0.2.0 or later when the factor changes. An e2e test with a fake agent whose sessions carry four times the expected bytes reaches the objective within 5 percent.

## Plan

- [x] 1. Claim commit and handoff
- [x] 2. Calibration loop in `controller/internal/runs/calibration.go`
- [x] 3. Wire it into the run loop and the run record
- [x] 4. E2E test with a fake agent that reacts to `UpdateRates`
- [x] 5. Wiki page, log entry, checks, PR

## Done and verified

Steps 1 to 4: `calibration.go`, wiring in `orchestrator.go`, unit and end to end tests, `mk/ctl.mk` targets, Controller page and log entry. CI on 9c9d3dc is green: workflow `ctl` ran golangci-lint and `go test -race ./...` (`internal/runs` ok). Go modules cannot be downloaded in the sandbox, so nothing ran locally.

## Remaining

Review. The coordinator adds `UpdateRates` to `agents.Manager` and `fakeagent` (CTL-06 paths, text in the log entry), or the calibration stays off on a real controller.

## Next action

Review PR #269.

## Blockers and open questions

`agents.Manager` has no `UpdateRates` method and `fakeagent` rejects the command. Both are outside the paths of this package. The orchestrator therefore reaches the command through an optional interface (`RateUpdater`).

## Decisions taken in this package

- The orchestrator reaches `UpdateRates` through the optional interface `RateUpdater`, because `agents.Manager` has no such method and `internal/agents/` is outside the paths of this package. The log entry holds the method for CTL-06.
- A mix entry maps to the `scenario` label of the client side counters, since the catalog has no mix entry label. Entries that share a scenario split its counters by weight.
- Per agent the command carries the sum of the compiled worker rates times the factor, so each command is absolute and a retry is harmless.

## Pitfalls

Go modules cannot be downloaded in the sandbox, so CI is the build loop.

## Verify the current state

```bash
cd controller && go test -race ./internal/runs/...
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-6555a2 | claim | none | Claimed #232 |
| 2026-11-09 | claude-6555a2 | checkpoint | 5e41c1d | Code and tests pushed, PR #269 opened |
| 2026-11-09 | claude-6555a2 | finish | 9c9d3dc | CI green, ready for review |
