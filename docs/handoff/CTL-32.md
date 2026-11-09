# Handoff CTL-32 Capture file retention in the controller

| Field | Value |
|---|---|
| Issue | #288 |
| Branch | `wp/CTL-32` |
| Pull request | #291 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-e61608, lease until 2026-11-09T15:12:51Z |
| Last verified commit | bc925b2 |

## Goal

CTL-14 stores downloaded capture files under the data directory and nothing removes them. Add a `capture_retention` setting (default 7 days) and a sweep that deletes capture files older than it. The run API has no delete operation, so no deletion on run delete.

## Plan

- [x] 1. `capture_retention` in `internal/app` config (default 168h, validated, optional flag).
- [x] 2. Sweep, injectable clock, ticker owned by the service, use guard for downloads and uploads in `internal/captures`.
- [x] 3. Wiring of the setting and shutdown in `internal/wiring`, tests, Controller wiki page, log entry.
- [x] 4. PR and CI green.

## Done and verified

All steps done. CI on bc925b2: ctl job (gofmt, go vet, golangci-lint, go test -race), controller-e2e, gate, and the rest are green. Go cannot build here, so only `gofmt -l` (clean), `plan.py check`, `render --check`, and `lint` ran locally; CI is the build and test loop.

## Remaining

none

## Next action

Review PR #291.

## Blockers and open questions

none

## Decisions taken in this package

- Age of a capture is the newest modification time below its directory, compared with the injected clock. This also covers directories that a restart orphaned, because the capture state lives in memory only.
- The run API has no delete operation (the contract has `deleteModule` and `deleteAgent` only), so there is no deletion hook for runs.

## Pitfalls

Go cannot build locally (no module proxy). CI is the build loop.

## Verify the current state

```bash
gofmt -l controller/internal/captures controller/internal/app controller/internal/wiring controller/cmd
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T13:43Z | claude-e61608 | claim | none | Claimed #288 |
| 2026-11-09T13:55Z | claude-e61608 | finish | bc925b2 | PR #291, CI green, ready for review |
