# Handoff CON-20 Stream events, metric source label, and provenance fields

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #377 |
| Branch | `wp/CON-20` |
| Pull request | none yet |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-13004c, lease until 2026-11-10T14:16:43Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Document the run SSE stream events (names, ids, retry, end event, 501) in the REST OpenAPI, add a source label to the mix metrics, and add optional provenance fields (capture_sha256, scrub_rules with versions, scrubbed_at). Additive only. Owner approved 2026-11-10.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read UI-04 and controller stream code; document stream events and 501 in the REST OpenAPI
- [x] 2. Add the source label to the mix metrics in the catalog; add provenance fields
- [x] 3. Version bumps, contracts/changes/CON-20.md, log entry, Contracts.md, run checks
- [ ] 4. PR and CI

## Done and verified

Steps 1 to 3. `python3 tools/contracts/validate.py` reports 0 errors and 0 warnings. `python3 -m pytest -q tools/contracts` gives 288 passed, 2 failed: the version assertion `test_catalog_is_0_3_0_with_property_both` and the byte copy test of the controller catalog, both outside `contracts/` (see Remaining). `plan.py check`, `render --check`, and `lint` pass. A jsonschema run accepted the example manifest and a manifest with the new fields, and rejected a bad digest and a rule without a version. oasdiff is not installed here, so the openapi-breaking workflow is the check for the additive rule.

## Remaining

Outside `contracts/` and left to the coordinator: refresh `controller/internal/metrics/catalog.yaml`, `APIVersion` 0.8.0 in `controller/internal/api/capabilities.go`, regenerate `api.gen.go`, update `engine/fps-stats/src/catalog.rs`, change the version assertion in `tools/contracts/test_contracts.py`, and regenerate the SDK and `ui/src/api/schema.d.ts`.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

The label `source` takes `metrics` and `classifier`, and a series without it counts as `metrics`, so no reader changes. The end event has no id and the controller ignores Last-Event-ID, as the code does. Provenance fields are optional for every source.

## Pitfalls

A catalog change breaks the byte copy test and the engine mirror test until the copies follow.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T12:50Z | claude-13004c | claim | | Claimed #377 |
| 2026-11-10T13:05Z | claude-13004c | checkpoint | | Contract changes, change entry, wiki, checks run |
