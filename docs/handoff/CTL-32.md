# Handoff CTL-32 Capture file retention in the controller

| Field | Value |
|---|---|
| Issue | #288 |
| Branch | `wp/CTL-32` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-e61608, lease until 2026-11-09T15:12:51Z |
| Last verified commit | none |

## Goal

CTL-14 stores downloaded capture files under the data directory and nothing removes them. Add a `capture_retention` setting (default 7 days) and a sweep that deletes capture files older than it. The run API has no delete operation, so no deletion on run delete.

## Plan

- [ ] 1. `capture_retention` in `internal/app` config (default 168h, validated, optional flag).
- [ ] 2. Sweep, injectable clock, ticker owned by the service, use guard for downloads and uploads in `internal/captures`.
- [ ] 3. Wiring of the setting and shutdown in `internal/wiring`, tests, Controller wiki page, log entry.
- [ ] 4. PR and CI green.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Implement step 1.

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
