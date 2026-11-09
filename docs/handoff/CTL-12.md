# Handoff CTL-12 Byte weighted mix control

| Field | Value |
|---|---|
| Issue | #164 |
| Branch | `wp/CTL-12` |
| Pull request | #0 |
| Status | in progress |
| Progress | 25% (1 of 4 plan steps) |
| Current holder | claude-c12a7e |
| Last verified commit | none |

## Goal

For a test with `profile.basis: bytes`, correct the per entry session rates every second until the byte share of each scenario is within 2 points of its weight. A simulation with fake agents (`fakeagent.Options.Traffic`) whose scenarios carry very different bytes per session converges within 30 seconds and stays within 2 points.

## Plan

- [x] 1. Claim commit, handoff, and the pure package `controller/internal/mixcontrol/` with unit tests (local `go test` works, the package uses only the standard library)
- [ ] 2. Compose with the CTL-24 calibration loop in `controller/internal/runs/calibration.go` (additive), per scenario bytes in `fakeagent.Traffic` (additive)
- [ ] 3. End to end test, `mk/ctl.mk` targets
- [ ] 4. Controller wiki page, log entry, checks, PR

## Done and verified

Step 1: `go test -race -cover ./internal/mixcontrol/` passes locally with 100 percent coverage.

## Remaining

Steps 2 to 4.

## Next action

Edit `controller/internal/runs/calibration.go` so the calibration loop owns a `mixcontrol.State` when `profile.basis` is `bytes`.

## Blockers and open questions

none

## Decisions taken in this package

- The schema already has `profile.basis` (`sessions` or `bytes`), so byte weighting is on exactly when the configuration says `bytes`. No contract change is needed. `runs.CalibrationConfig.ByteMixOff` turns it off.
- One loop per run sends `UpdateRates`: the CTL-24 calibration loop. `mixcontrol` is a pure estimator and controller. It returns a multiplier per entry that preserves the compiled total bytes per second, so the calibration factor (which scales the total to the objective) and the multipliers (which move bytes between entries) compose by multiplication, and the loop still sends one command per agent per window.
- The compiler normalizes the mix by sessions whatever the basis ("Byte based mix correction arrives in CTL-12"), so for now the compiled session weight and the byte weight of an entry are the same number. `mixcontrol.Entry` keeps them apart, so a later compiler change needs no change here.

## Pitfalls

The sandbox cannot build `internal/runs` (modules do not download), so CI is the loop for step 2 onward.

## Verify the current state

```bash
cd controller && GOWORK=off GOFLAGS=-mod=mod go test -race -cover ./internal/mixcontrol/
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T15:12Z | claude-c12a7e | claim | none | Claimed #164 |
