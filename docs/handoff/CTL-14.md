# Handoff CTL-14 Captures API

| Field | Value |
|---|---|
| Issue | #166 |
| Branch | `wp/CTL-14` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-7c949b, lease until 2026-11-09T14:08:05Z |
| Last verified commit | none |

## Goal

Start filtered captures on the agents of a run, serve the pcapng files: POST and GET /v1/runs/{id}/captures and the capture download, with fetch_capture and UploadCapture (CON-13).

## Plan

- [ ] 1. Agent manager: StartCapture, StopCapture, FetchCapture sends and the UploadCapture server method (agents/capture.go)
- [ ] 2. captures package: service, state, file store with resume and digest check
- [ ] 3. REST handlers and wiring
- [ ] 4. fakeagent capture support (fakeagent/capture.go)
- [ ] 5. Tests: contract-style tests with a fake agent, resume, digest mismatch
- [ ] 6. Docs: Controller.md, log entry, CI green

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write controller/internal/agents/capture.go.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

Go cannot build locally (no module proxy); CI is the build loop.

## Verify the current state

```bash
cd controller && go test ./internal/captures/ ./internal/agents/
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-7c949b | claim | | claimed #166 |
