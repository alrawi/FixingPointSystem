# Handoff CON-16 Controller version, configuration listing, and REST breaking check

| Field | Value |
|---|---|
| Issue | #313 |
| Branch | `wp/CON-16` |
| Pull request | #0 |
| Status | in progress |
| Progress | 80% (2 of 3 plan steps) |
| Current holder | claude-c16a7e |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the optional `Capabilities.controller_version`, `GET /configs` (`listConfigs`, paged like `listRuns`), the "session share" comment of `MixEntry.weight`, and an oasdiff breaking check for the OpenAPI documents with a deliberately breaking fixture. All additive, with `contracts/changes/CON-16.md`.

## Plan

- [x] 1. OpenAPI 0.7.0, proto comment, change entry, changelog, wiki page, generated Python and UI code.
- [x] 2. Workflow `openapi-breaking.yml`, `check_openapi_breaking.py`, fixtures, fixture test.
- [ ] 3. Open the PR, get CI green (oasdiff pin and OpenAPI 3.1 support are unverified locally).

## Done and verified

- `python3 tools/contracts/validate.py`: 0 errors. `generate_client.py` and `npm run gen:api` ran.
- The fixture test is skipped locally, because oasdiff cannot be downloaded in the sandbox (no access to GitHub releases or the Go module proxy). The workflow runs it.

## Remaining

CI on the PR. The coordinator regenerates `controller/internal/api/api.gen.go`.

## Next action

Poll the checks of the PR. If `openapi-breaking` fails at `go install`, fix the pinned version in the workflow.

## Blockers and open questions

none

## Decisions taken in this package

- The protocol stays at 0.2.3: a comment-only change alters no wire behavior (see `contracts/changes/CON-16.md`).
- oasdiff is built with `go install` at a pinned version, which the Go checksum database verifies, instead of downloading a binary.

## Pitfalls

The sandbox cannot run oasdiff, so the fixtures were not run locally.

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest -q tools/contracts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T20:57Z | claude-c16a7e | claim | none | Claimed #313 |
