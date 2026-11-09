# Handoff CLI-02 fps command line client

| Field | Value |
|---|---|
| Issue | #169 |
| Branch | `wp/CLI-02` |
| Pull request | #306 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-c02f7a, lease until 2026-11-09T17:42:54Z |
| Last verified commit | 12e418d |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A Go command line client `fps` in the new module `cli/` (standard library only) for configurations, runs, live metrics, and reports of the controller REST API, with end to end tests against a real controller with fake agents.

## Plan

- [x] 1. Module, HTTP client, global flags, auth, exit codes, output
- [x] 2. `config`, `run`, `report` commands
- [x] 3. `watch` command (SSE with polling fallback) and unit tests against a stub server
- [x] 4. Test helper `controller/cmd/fps-fakeagent` and the end to end test
- [x] 5. `mk/cli.mk`, wiki page `docs/wiki/CLI.md`, log entry, pull request, CI

## Done and verified

- `cd cli && go test -race ./...` with `GOWORK=off GOFLAGS=-mod=mod GOPROXY=off`: the unit tests pass (84% coverage of `internal/fpscli`), and the end to end test skips because the controller cannot build offline.
- `golangci-lint run` in `cli/`: 0 issues. `make lint-cli test-cli` pass offline.
- `plan.py check`, `render --check`, and `lint`: ok.

## Remaining

Nothing. The workflow `cli-go` passed on 12e418d, and `TestEndToEnd` ran for about 30 seconds against the real controller and the fake agent.

## Next action

Review pull request #306.

## Blockers and open questions

none

## Decisions taken in this package

- The API has no operation that lists configurations, so `fps config list` shows the configurations that this client uploaded, from a local history file.
- `fps config validate FILE` stores the configuration first, because the API has no dry run.
- The token comes from a file or the environment only, never from a flag, so it does not show in the process list.
- `fps run list` reports the 501 of the controller (listRuns is pending in CTL wiring) with exit code 5.

## Pitfalls

- The controller module cannot build in the sandbox (no module proxy), so the end to end test and the fake agent helper only run in CI.

## Verify the current state

```bash
cd cli && GOWORK=off GOFLAGS=-mod=mod GOPROXY=off go test ./...
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T17:13Z | claude-c02f7a | claim | none | Claimed issue #169 |
| 2026-11-09T17:45Z | claude-c02f7a | checkpoint | 1ccbd4a | Client, tests, fake agent helper, make targets; docs next |
| 2026-11-09T18:00Z | claude-c02f7a | review | b52a979 | Pull request #306 opened, ready for review |
| 2026-11-09T18:30Z | claude-c02f7a | checkpoint | 12e418d | Fixed the gosec G705 finding and the failed-criterion assertion; cli-go green |
