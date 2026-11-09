# Handoff CTL-36 Serve stopCapture, capture files, and labeled criteria

| Field | Value |
|---|---|
| Issue | #323 |
| Branch | `wp/CTL-36` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-c7a1e4, lease until 2026-11-09T21:23:49Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The controller serves `stopCapture`, fills `Capture.files` from the per agent results, passes `export_tls_keys` to the agents, and sends labeled criteria only to agents that report engine_api 0.2.3 or later. `TestDownloadResumesAfterAnInterruptedUpload` becomes stable.

## Plan

- [ ] 1. captures: `Service.Stop`, `API.StopCapture`, `Capture.files`, `export_tls_keys` in the command; wiring routes `stopCapture`.
- [ ] 2. compiler: `Criterion.labels` from `where`, refusal for agents below 0.2.3; agents: gate in `Prepare`.
- [ ] 3. Fix the flake in the fake agent break path; stress run in CI.
- [ ] 4. Docs: `docs/wiki/Controller.md`, log entry.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Step 1.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

Go modules cannot be downloaded in the sandbox. CI is the build loop.

## Verify the current state

```bash
cd controller && GOWORK=off go test -race ./internal/captures/ ./internal/compiler/ ./internal/agents/ ./internal/wiring/
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T20:53Z | claude-c7a1e4 | claim | - | claimed #323 |
