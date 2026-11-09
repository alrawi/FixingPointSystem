# Handoff CLI-02 fps command line client

| Field | Value |
|---|---|
| Issue | #169 |
| Branch | `wp/CLI-02` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-c02f7a, lease until 2026-11-09T17:42:54Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A Go command line client `fps` in the new module `cli/` (standard library only) for configurations, runs, live metrics, and reports of the controller REST API, with end to end tests against a real controller with fake agents.

## Plan

- [ ] 1. Module, HTTP client, global flags, auth, exit codes, output
- [ ] 2. `config`, `run`, `report` commands
- [ ] 3. `watch` command (SSE with polling fallback) and unit tests against a stub server
- [ ] 4. Test helper `controller/cmd/fps-fakeagent` and the end to end test
- [ ] 5. `mk/cli.mk`, wiki page `docs/wiki/CLI.md`, log entry, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write the module skeleton under `cli/`.

## Blockers and open questions

none

## Decisions taken in this package

- The API has no operation that lists configurations, so `fps config list` shows the configurations that this client uploaded, from a local history file.

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
