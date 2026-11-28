# Handoff CTL-55 The controller shows the backend that ran

| Field | Value |
|---|---|
| Issue | #681 |
| Branch | `wp/CTL-55` |
| Pull request | #700 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | see the card on #681 |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Store the backend that ran each plan and the bound XDP mode of each port from the fields of CON-35, show them in the run report and the API, and mark a run whose backend differs from the one the inventory listed.

## Plan

- [x] 1. Agent manager keeps run_backend and run_ports and emits an event on change
- [x] 2. Orchestrator keeps them per agent of a run with the selected backend, REST `Run.agents`, CSV and HTML report marks
- [x] 3. Tests, Controller page, log entry, this file
- [x] 4. Pull request and CI

## Done and verified

All steps done. Go cannot build in the sandbox, so only `gofmt -l` ran locally. PR #700 CI is green on all check runs.

## Remaining

Nothing.

## Next action

The coordinator reviews and merges #700.

## Blockers and open questions

none

## Decisions taken in this package

- `BACKEND_UNSPECIFIED` is unknown. The run lists no backend for that agent, so it is never shown as kernel. The CSV and HTML reports list it as `unknown` once another agent of the run reported.
- The JSON report has no mismatch property in the contract, so the mismatch shows as `backend` next to `selected_backend` of `Run.agents`. The CSV and HTML reports add an explicit mark.
- The manager emits `EventStateChanged` also when only the run fields change, so a run learns them if they arrive one heartbeat after the state change.
- The controller keeps the newest known backend: a later heartbeat without one does not clear it.

## Pitfalls

- HTML goldens stay unchanged only because the Backend note sits on the line of the failed agents note and prints nothing without backends.

## Verify the current state

```bash
cd controller && gofmt -l . && go test ./internal/agents ./internal/runs ./internal/report/...
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-27 | worker | claim and implementation | none | lease on #681 |
