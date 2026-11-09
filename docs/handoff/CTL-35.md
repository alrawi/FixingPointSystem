# Handoff CTL-35 List runs

| Field | Value |
|---|---|
| Issue | #312 |
| Branch | `wp/CTL-35` |
| Pull request | #314 |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-c35a7e, lease until 2026-11-09T19:41:21Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement `listRuns` (`GET /v1/runs`) as the OpenAPI contract defines it: filter `state`, `page_size` (1 to 500, default 50), `page_token`, response `{items, next_page_token}`, newest first. `fps run list` must then exit 0. Go cannot build here, so the pull request CI is the build loop.

## Plan

- [x] 1. Orchestrator.List with a stable order and a robust page token; API.ListRuns in `controller/internal/runs`.
- [x] 2. Wire it: drop the pending stub in `controller/internal/wiring/server.go` and `listRuns` from `pendingOps`; contract tests.
- [x] 3. CLI e2e test expects exit 0 from `fps run list` and the started runs.
- [ ] 4. Docs (Controller.md, CLI.md), log entry, checks, pull request, CI.

## Done and verified

Steps 1 to 3 written. `gofmt -l`, `plan.py check`, `render --check`, and `lint` pass locally. Go cannot build here, so the PR CI is the build and test loop.

## Remaining

CI is red on the first push with no job or log visible through the API (zero jobs listed), rerun did the same. Retriggering with a new commit.

## Next action

Watch CI of the newest commit; fix failures inside the package paths.

## Blockers and open questions

none

## Decisions taken in this package

- Order is a per run creation sequence, newest first, assigned under the orchestrator lock. It does not depend on the clock, so equal timestamps stay stable.
- The page token holds the sequence of the last item of the page, and the next page holds the runs with a lower sequence. Runs added between pages get a higher sequence, so they never shift or repeat a page.

## Pitfalls

Go modules cannot be downloaded here, so nothing compiles locally.

## Verify the current state

```bash
gofmt -l controller cli
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T18:11Z | claude-c35a7e | claim | | Claimed #312 |
| 2026-11-09T18:25Z | claude-c35a7e | checkpoint | f44ebce | PR #314 opened; ctl and cli-go failed with no visible jobs or logs, rerun failed the same way |
