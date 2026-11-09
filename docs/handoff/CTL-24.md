# Handoff CTL-24 Throughput calibration in the run loop

| Field | Value |
|---|---|
| Issue | #232 |
| Branch | `wp/CTL-24` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-6555a2, lease until 2026-11-09T05:23:25Z |
| Last verified commit | none |

## Goal

For a `throughput_bps` objective, the orchestrator builds a `calibrate.State` from the compiled mix and `expected_bytes`, feeds it the client side bytes and completed sessions per mix entry every measurement window, and sends `UpdateRates` (CON-05) to agents with `engine_api` 0.2.0 or later when the factor changes. An e2e test with a fake agent whose sessions carry four times the expected bytes reaches the objective within 5 percent.

## Plan

- [ ] 1. Claim commit and handoff
- [ ] 2. Calibration loop in `controller/internal/runs/calibration.go`
- [ ] 3. Wire it into the run loop and the run record
- [ ] 4. E2E test with a fake agent that reacts to `UpdateRates`
- [ ] 5. Wiki page, log entry, checks, PR

## Done and verified

Nothing yet.

## Remaining

Everything after the claim.

## Next action

Write `calibration.go`.

## Blockers and open questions

`agents.Manager` has no `UpdateRates` method and `fakeagent` rejects the command. Both are outside the paths of this package. The orchestrator therefore reaches the command through an optional interface (`RateUpdater`).

## Decisions taken in this package

None yet.

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
