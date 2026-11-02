# Handoff CON-02 Counter catalog v0.1

| Field | Value |
|---|---|
| Issue | #18 |
| Branch | `wp/CON-02` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-con02-9dd7, lease until 2026-11-02T18:58:01Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Name every counter, gauge, and histogram once in `contracts/metrics/catalog.yaml`, with type, unit, level, labels, and description, so that the agent, the controller, the API, and the UI agree on metric names. A validator rule rejects metric names in example test criteria and in the OpenAPI document that the catalog does not define.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Write the catalog JSON Schema and `contracts/metrics/catalog.yaml` for the four levels of the specification.
- [ ] 2. Add catalog checks and the metric reference rule to `tools/contracts/validate.py`, and annotate the OpenAPI run summary with catalog names.
- [ ] 3. Add tests, including one that shows an unknown metric fails.
- [ ] 4. Update `contracts/README.md` and `contracts/CHANGELOG.md`.
- [ ] 5. Ingest into the wiki, append the Log entry, and finish.

## Done and verified

None yet.

## Remaining

All plan steps.

## Next action

Write `contracts/metrics/catalog.schema.json`.

## Blockers and open questions

None.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 -m pip install -r tools/contracts/requirements.txt
python3 tools/contracts/validate.py     # expect 0 errors
python3 -m pytest -q tools/contracts     # expect all tests to pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T17:28:01Z | claude-con02-9dd7 | claim | none | Fresh claim, no earlier cards |
