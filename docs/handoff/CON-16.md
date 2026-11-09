# Handoff CON-16 Controller version, configuration listing, and REST breaking check

| Field | Value |
|---|---|
| Issue | #313 |
| Branch | `wp/CON-16` |
| Pull request | #326 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-c16a7e |
| Last verified commit | 5098231 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the optional `Capabilities.controller_version`, `GET /configs` (`listConfigs`, paged like `listRuns`), the "session share" comment of `MixEntry.weight`, and an oasdiff breaking check for the OpenAPI documents with a deliberately breaking fixture. All additive, with `contracts/changes/CON-16.md`.

## Plan

- [x] 1. OpenAPI 0.7.0, proto comment, change entry, changelog, wiki page, generated Python and UI code.
- [x] 2. Workflow `openapi-breaking.yml`, `check_openapi_breaking.py`, fixtures, fixture test.
- [x] 3. PR #326 open. `openapi-breaking` is green. The Go jobs fail only until the coordinator regenerates `api.gen.go` (`undefined: ListConfigsParams`).

## Done and verified

- `python3 tools/contracts/validate.py`: 0 errors. `generate_client.py` and `npm run gen:api` ran.
- CI `openapi-breaking` green at 5098231: fixture tests pass, and the base comparison reports no breaking change. The fixture test is skipped locally, because oasdiff cannot be downloaded in the sandbox (no access to GitHub releases or the Go module proxy). The workflow runs it.

## Remaining

The coordinator regenerates `controller/internal/api/api.gen.go`.

## Next action

Coordinator: run `go generate ./internal/api`, push it to the branch, and merge when green.

## Blockers and open questions

none

## Decisions taken in this package

- The protocol stays at 0.2.3: a comment-only change alters no wire behavior (see `contracts/changes/CON-16.md`).
- oasdiff cannot load the JSON Schema files (numeric `exclusiveMinimum`), so the check replaces `../schemas/*.json` refs by an opaque object.
- oasdiff is built with `go install` at a pinned version, which the Go checksum database verifies, instead of downloading a binary.

## Pitfalls

The sandbox cannot run oasdiff, so only CI ran the fixtures.

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest -q tools/contracts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T20:57Z | claude-c16a7e | claim | none | Claimed #313 |
| 2026-11-09T21:05Z | claude-c16a7e | review | 5098231 | PR #326 |
