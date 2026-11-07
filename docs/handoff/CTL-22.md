# Handoff CTL-22 Module settings and port locations end to end in the controller

| Field | Value |
|---|---|
| Issue | #218 |
| Branch | `wp/CTL-22` |
| Pull request | #237 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-b67d88, lease see the status card on #218 |
| Last verified commit | 92c8a82 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Close the controller gaps that CTL-18 reported outside its paths. The catalog adapter fills `Scenario.SettingsSchema` from the manifest `settings` key, and `internal/store/validate.go` parses port locations as the compiler does and covers `modules` in its compiler path pattern. The pending examples must compile through the store and the API.

## Plan

- [x] 1. Fill `SettingsSchema` in `internal/wiring/adapters.go`.
- [x] 2. Parse locations in `internal/store/validate.go` as the compiler does and add `modules` to `compilerPathRE`.
- [x] 3. Tests in both packages that read `contracts/examples/tests-pending/`, and the Controller page.
- [x] 4. Push, wait for CI, fix failures, open the pull request.

## Done and verified

Plan checks and gofmt ran locally. CI on 92c8a82 passed, including the ctl test job.

## Remaining

Nothing.

## Next action

Coordinator review of pull request 237.

## Blockers and open questions

none

## Decisions taken in this package

- `splitLocation` in `store` mirrors the compiler parser instead of importing it, because `internal/compiler` is outside this package. A table test pins the forms.
- `agents_required` counts distinct pinned agents, or one when only spread ports exist. `agents_available` counts the available agents that a port can use.
- The dry run takes the agents that the ports can use as its inventory, which includes every agent that a spread port reaches.
- `compilerPathRE` accepts digits and hyphens in path segments, so `modules.http1.settings` maps to `/modules/http1/settings`.

## Pitfalls

The worktree guard refuses compound shell commands that mix git with other commands, so run git alone.

## Verify the current state

```bash
cd controller && gofmt -l .
# CI runs: make lint-ctl test-ctl
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-07 | claude-b67d88 | claim | none | Claimed #218 and wrote steps 1 to 3 |
