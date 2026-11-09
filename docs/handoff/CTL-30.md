# Handoff CTL-30 Send UpdateRates from the agent manager

| Field | Value |
|---|---|
| Issue | #274 |
| Branch | `wp/CTL-30` |
| Pull request | #280 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-c30a7e, lease until 2026-11-09T14:07:22Z |
| Last verified commit | none |

## Goal

`agents.Manager` sends `UpdateRatesCommand` and returns `UpdateRatesResult` like `Start`, `fakeagent` accepts UpdateRates while running and changes the session rate it simulates, and the CTL-24 test runs against the real manager and fakeagent without its `rateManager` wrapper. A controller e2e test with fakeagent shows calibration steps in `Run.Calibration` for a `throughput_bps` objective.

## Plan

- [x] 1. `Manager.UpdateRates` in a new file in `internal/agents/`.
- [x] 2. `fakeagent` UpdateRates handling and an optional traffic simulation.
- [x] 3. CTL-24 test without the wrapper, plus a manager test.
- [x] 4. Controller.md, log entry, checks, PR.

## Done and verified

Steps 1 to 4 done; CI green (all 11 check runs). `gofmt -l controller` is clean, and `plan.py check`, `render --check`, and `lint` pass. Go cannot build in the sandbox, so CI is the first compile and test run.

## Remaining

Nothing; CI is green on the PR.

## Next action

Review and merge PR #280.

## Blockers and open questions

none

## Decisions taken in this package

- The manager method sits in a new file, not in `service.go`, so CTL-14 can add capture code in parallel without merge conflicts.
- `wiring` already passes `*agents.Manager` to `runs.Config.Agents`, so the new method switches the calibration on with no wiring edit.

## Pitfalls

Go modules cannot be downloaded in the sandbox, so CI is the build loop.

## Verify the current state

```bash
gofmt -l controller/
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-c30a7e | claim | none | claimed #274 |
