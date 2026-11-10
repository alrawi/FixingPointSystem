# Handoff CTL-41 Realism reports attached to a run

| Field | Value |
|---|---|
| Issue | #421 |
| Branch | `wp/CTL-41` |
| Pull request | none yet |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-8d6916, lease until 2026-11-11T00:05:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

CON-22 added `PUT /v1/runs/{id}/realism` (attachRealismReport), which answers 501 today. The operation stores the realism report of a finished run (replace on repeat, 409 for a run still active, 413 above 4 MiB, 415, 422 against an embedded copy of `realism-report.schema.json`), maps each check metric to a `RealismResult` as `contracts/changes/CON-22.md` says, and fills `RunReport.realism`.

## Plan

- [x] 1. Embedded schema copy, validation with JSON Pointer findings, and the metric mapping (`controller/internal/report/realism.go`) with unit tests.
- [x] 2. The in memory per run store and the handler (`AttachRealismReport`) with the 400, 404, 409, 413, 415, 422 cases.
- [x] 3. `Report.Realism` filled by `GetRunReport`; wiring (the operation leaves `pendingOps`).
- [x] 4. REST tests with literal results for a conformance and a temporal check, replace and refusal cases, the byte identical schema copy test.
- [ ] 5. Checks, wiki page, log entry, pull request, CI.

## Done and verified

- Steps 1 to 4 are written. Go cannot build here, so CI is the first compile and test run. `gofmt -l controller` prints nothing, and `testdata/realism-report.json` validates against the schema with python jsonschema (0 errors).

## Remaining

Step 5: local checks (plan.py, lint), pull request, CI fixes.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

- Reports live in `report.MemoryRealism` (a `RealismStore`), because the orchestrator keeps runs in memory, so a persisted report would outlive its run.
- `ReportHandler` (wiring) now requires `AttachRealismReport`; `pending.AttachRealismReport` forwards to it, like `GetRunReport`, so `SetReport` swaps both operations together and the attached report is the one that is read.
- Check order of the handler: 404, 409, 415, 413, 400, 422.
- A metric that finds no application or comparison to take its outcome from gets the generic `check passed` rule. A value that overflows float32 is a 422 finding, because JSON encoding of an infinite float32 would break `getRunReport`.
- `html.Renderer` fills the realism rows of the HTML report from `Report.Realism` (inside `report/`).

## Pitfalls

Go cannot build in the sandbox (no module proxy), so CI is the build loop. Keep every change gofmt clean.

## Verify the current state

```bash
cd controller && gofmt -l .        # prints nothing
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T22:35Z | claude-8d6916 | claim | none | Claimed issue 421, branch wp/CTL-41 |
| 2026-11-10T23:05Z | claude-8d6916 | checkpoint | see branch | Steps 1 to 4 written |
