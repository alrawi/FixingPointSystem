# Handoff CTL-37 Serve listConfigs and the controller version

| Field | Value |
|---|---|
| Issue | #339 |
| Branch | `wp/CTL-37` |
| Pull request | #342 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-ctl37a |
| Last verified commit | bbe787b |

## Goal

Serve `listConfigs` (`GET /v1/configs`) newest first with the contract's paging, report `controller_version` from the CTL-33 version stamp, and make `fps config list` use the operation, keeping the local history only as a fallback for a controller that answers 501.

## Plan

- [x] 1. Store: creation sequence (schema version 2), `Repository.List`, `Handler.ListConfigs`, tests against the OpenAPI document.
- [x] 2. Capabilities: `api.WithControllerVersion`, `wiring.Options.Version`, `buildVersion()` from main; wiring stub and `pendingOps` lose `listConfigs`.
- [x] 3. CLI: `fps config list` uses `GET /v1/configs`, history fallback on 501; unit tests and `cli/e2e`.
- [x] 4. Wiki pages, log entry, CI green, review.

## Done and verified

- `cd cli && GOWORK=off go test ./internal/...` passes locally (the CLI module has no external dependencies).
- Controller Go code cannot build here (no module proxy); `gofmt -l` is clean. CI on bbe787b: controller `test`, `controller-e2e`, cli `test` (with `cli/e2e`), contracts, gate (after one re-run) green.

## Remaining

Nothing. CI on bbe787b is green (the gate job failed once on a one-off tshark/zeek parser finding in http1 traffic, unrelated, and passed on re-run).

## Next action

Review and merge PR #342.

## Blockers and open questions

none

## Decisions taken in this package

- Paging follows `listRuns` (CTL-35): opaque token `base64url("c" + seq)` of the last item, one row more than the page is read to decide whether a token follows.
- The creation order is a new column `seq INTEGER PRIMARY KEY AUTOINCREMENT` (migration to user_version 2), not the implicit rowid, because rowids of a table without an integer primary key can change on `VACUUM`. The migration copies old rows in rowid order.
- `controller_version` is `buildVersion()` of main, which is the stamp, else the module version, else `(devel)`; it is passed through `api.WithControllerVersion` and `wiring.Options.Version` (wiring is outside the paths, allowed by the brief).
- The CLI falls back to the history only when the first request answers 501. `--all-controllers` stays a history-only listing.

## Pitfalls

Controller packages cannot be compiled locally.

## Verify the current state

```bash
cd cli && GOWORK=off go test ./internal/...   # ok
gofmt -l controller cli                        # no output
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T23:15Z | claude-ctl37a | claim | none | claimed #339 |
| 2026-11-09T23:30Z | claude-ctl37a | checkpoint | bbe787b | PR 342 opened, CI fixes pushed |
| 2026-11-09T23:40Z | claude-ctl37a | review | bbe787b | CI green, ready for review |
