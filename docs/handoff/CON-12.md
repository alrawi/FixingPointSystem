# Handoff CON-12 Side selection in the run metrics API

| Field | Value |
|---|---|
| Issue | #242 |
| Branch | `wp/CON-12` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-fd99b8, lease until 2026-11-09T04:06:55Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the optional `side` query parameter (client or server, default client) to the run metrics endpoint and its stream, additively, with a change note, and serve the server side in the controller.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. OpenAPI parameter, version 0.4.0, change note, wiki
- [x] 2. Regenerate the UI schema and the Python SDK
- [x] 3. Handler and tests
- [ ] 4. CI green, regeneration of api.gen.go by the coordinator

## Done and verified

See the PR description.

## Remaining

`api.gen.go` regeneration (coordinator).

## Next action

Wait for the coordinator to push the regenerated `api.gen.go`, then check CI.

## Blockers and open questions

none

## Decisions taken in this package

Minor bump to 0.4.0 for an added parameter. The stream takes `side` too, because it shares the views and the aggregator.

## Pitfalls

none

## Verify the current state

```bash
python3 tools/contracts/validate.py
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T03:10:00Z | claude-fd99b8 | claim and first checkpoint | | contract, handler, tests written |
