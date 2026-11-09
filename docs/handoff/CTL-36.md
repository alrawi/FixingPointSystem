# Handoff CTL-36 Serve stopCapture, capture files, and labeled criteria

| Field | Value |
|---|---|
| Issue | #323 |
| Branch | `wp/CTL-36` |
| Pull request | #327 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-c7a1e4, lease until 2026-11-09T21:23:49Z |
| Last verified commit | 985b89f (ctl job green), 50b8a3a removes the temporary workflow |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The controller serves `stopCapture`, fills `Capture.files` from the per agent results, passes `export_tls_keys` to the agents, and sends labeled criteria only to agents that report engine_api 0.2.3 or later. `TestDownloadResumesAfterAnInterruptedUpload` becomes stable.

## Plan

- [x] 1. captures: `Service.Stop`, `API.StopCapture`, `Capture.files`, `export_tls_keys` in the command; wiring routes `stopCapture`.
- [x] 2. compiler: `Criterion.labels` from `where`, refusal for agents below 0.2.3; agents: gate in `Prepare`.
- [x] 3. Fix the flake in the fake agent break path (stress run in CI pending).
- [x] 4. Docs: `docs/wiki/Controller.md`, log entry.

## Done and verified

Code, tests, docs, and log entry written. The ctl job of CI is green on 985b89f. A temporary stress workflow ran the flaky test 200 times under load with -race: the old fake agent failed once (attempt 2 starts at offset 400, the controller held 500), the fixed one passed 200 of 200 and the whole captures package passed 20 times. The workflow is removed in 50b8a3a.

## Remaining

Review.

## Next action

Review PR #327.

## Blockers and open questions

none

## Decisions taken in this package

- Labeled criteria are refused in `compiler.Compile` (agents with a known engine_api below 0.2.3) and again in `agents.Manager.Prepare`. Empty compiler `Agent.EngineAPI` means unknown and passes, so the validation inventory stays free of versions.
- `stopCapture` answers 409 when an agent of a running run cannot answer, because the capture would be neither running nor ended; 200 otherwise.
- The flake lives in `fakeagent/capture.go` (cancel after 50 ms sleep); fixed there with a half close. Outside the catalog paths, needed by the acceptance.

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
| 2026-11-09T20:10Z | claude-c7a1e4 | finish | 50b8a3a | PR #327, ctl green |
