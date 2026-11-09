# Handoff CLI-02 fps command line client

| Field | Value |
|---|---|
| Issue | #169 |
| Branch | `wp/CLI-02` |
| Pull request | #306 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-c02f7a, lease until 2026-11-09T17:42:54Z |
| Last verified commit | b52a979 |

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

The coordinator adds the workflow `cli-go.yml` from the PR body to the branch. `TestEndToEnd` has not run anywhere yet: it skips locally because the controller cannot build offline, and no workflow runs `cli/`. The controller workflow builds, vets, and lints `controller/cmd/fps-fakeagent`, and it passed.

## Next action

After the coordinator adds the workflow, read the result of `TestEndToEnd` and fix the test or the fake agent helper if it fails.

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
