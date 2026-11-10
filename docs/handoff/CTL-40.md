# Handoff CTL-40 The first response frame of an agent session

| Field | Value |
|---|---|
| Issue | #420 |
| Branch | `wp/CTL-40` |
| Pull request | #431 |
| Status | in progress |
| Progress | 90% (2 of 3 plan steps) |
| Current holder | claude-463d82, lease until 2026-11-11T00:05:31Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`Manager.Session` reads the open message, marks the agent online, and sends nothing until it has a command or a heartbeat to ack. The Rust agent waits for the first response frame (the headers) before it heartbeats, so an idle session is timed out 5 to 6 s after it began (IMG-07). The controller must send the response headers as soon as the session opens, and a test client that waits for the headers before it heartbeats must hold an idle session open for three heartbeat timeouts, and fail on the old code.

## Plan

- [x] 1. Add the regression test with a client that waits for the response headers, push it alone, and let CI show that it fails on the old code.
- [x] 2. Send the response headers after `m.open` in `Manager.Session` and push; CI shows the test passing.
- [ ] 3. Wiki: log entry and Controller page, open the pull request, wait for CI.

## Done and verified

- Step 1: test commit b55c691 failed in CI on the old code (run 42916193523, job 130179128966): `the controller sent no response headers within 2 s ... Sweep ... ended [agent-h]`. Every other controller package passed.
- Step 2: fix and wiki in the next commit; `gofmt -l controller/internal/agents/` clean, `plan.py check`, `render --check`, `lint` pass locally. Go tests run only in CI.

## Remaining

CI result of the fix commit.

## Next action

Wait for CI on the fix commit, then set the status to ready for review and run `fps_swarm.py review 420 --pr 431`.

## Blockers and open questions

none

## Decisions taken in this package

- The test client lives in `controller/internal/agents/` (a test file), because `internal/fakeagent/` is outside the paths of this package. It talks to the generated client directly and calls `stream.Header()`, which blocks until the response headers arrive, as tonic does inside `client.session(...)`.

## Pitfalls

- Go cannot build locally (no module proxy), so CI is the build loop.

## Verify the current state

```bash
gofmt -l controller/internal/agents/
# CI: job of .github/workflows/ctl.yml on the pull request
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T00:06Z | claude-463d82 | claim | none | Claimed #420, branch wp/CTL-40 from origin/main 87bb307 |
| 2026-11-10T22:50Z | claude-463d82 | checkpoint | b55c691 | Test commit failed on old code in CI; fix pushed next |
