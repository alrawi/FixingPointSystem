# Handoff CTL-14 Captures API

| Field | Value |
|---|---|
| Issue | #166 |
| Branch | `wp/CTL-14` |
| Pull request | none yet |
| Status | in progress |
| Progress | 80% (5 of 6 plan steps) |
| Current holder | claude-7c949b, lease until 2026-11-09T14:08:05Z |
| Last verified commit | none (CI pending) |

## Goal

Start filtered captures on the agents of a run, serve the pcapng files: POST and GET /v1/runs/{id}/captures and the capture download, with fetch_capture and UploadCapture (CON-13).

## Plan

- [x] 1. Agent manager: StartCapture, StopCapture, FetchCapture sends and the UploadCapture server method (agents/capture.go)
- [x] 2. captures package: service, state, file store with resume and digest check
- [x] 3. REST handlers and wiring
- [x] 4. fakeagent capture support (fakeagent/capture.go)
- [x] 5. Tests: contract-style tests with a fake agent, resume, digest mismatch
- [ ] 6. Docs: Controller.md, log entry, CI green

## Done and verified

Code, tests, docs written. Not built locally (no Go module proxy); CI is the build loop. Not yet verified by CI.

## Remaining

CI green; fix compile or lint errors that CI reports; open and finish the PR.

## Next action

Open the PR and read the CI result of the controller workflow.

## Blockers and open questions

none

## Decisions taken in this package

- Download ends a running capture (the contract has no stop operation). Multiple agents: the files are concatenated, which pcapng allows; headers name present and missing agents.
- Plan id of an agent is `<run id>/<agent id>` (compiler), so the service needs nothing from `runs` beyond `Run.Agents` and `FailedAgents`.
- State is in memory like runs; files are under `<data_dir>/captures`, no retention yet.
- `Manager.UploadCapture` waits for the fetch answer (`ready`), because the answer and the call travel on different streams.

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
