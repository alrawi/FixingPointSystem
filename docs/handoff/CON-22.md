# Handoff CON-22 Realism report delivery and reservation conflicts in the REST API

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #406 |
| Branch | `wp/CON-22` |
| Pull request | #0 |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-73ec6a, lease until 2026-11-10T18:55:01Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

An additive REST contract change: an operation that attaches a realism report to a run, a documented 409 for reserveAgent, and an optional Agent.reserved_by. Stay inside contracts/ and list the controller, UI, SDK, and CLI follow up for the coordinator.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read the report schema, CON-21, and the controller reserve code, and decide the contract
- [x] 2. Edit the OpenAPI document (0.10.0)
- [x] 3. Write the change entry, the Contracts page section, and the log entry
- [ ] 4. Open the PR, wait for CI, and hand over for review

## Done and verified

`python3 tools/contracts/validate.py` 0 errors; `python3 -m pytest -q tools/contracts` 290 passed, 1 skipped (the oasdiff test skips without oasdiff); `plan.py check`, `render --check`, and `lint` ok.

## Remaining

oasdiff could not run locally (not installed, no module proxy), so the openapi-breaking workflow is its first run. Controller, SDK, and UI follow up is the coordinator's (see contracts/changes/CON-22.md).

## Next action

Open the PR and watch openapi-breaking and the contract checks.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

PUT replaces; only ended runs accept; 4 MiB; request body is a $ref to the realism report schema; mapping per metric in CON-22.md; response is an object (RealismAttachment) so it can grow. Version 0.10.0 (minor).

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T17:25Z | claude-73ec6a | claim | - | claimed #406 |
| 2026-11-10T17:50Z | claude-73ec6a | checkpoint | - | contract, change entry, wiki page, log entry written; checks green locally |
