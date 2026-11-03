# Handoff CON-09 Test configuration and manifest schema additions

| Field | Value |
|---|---|
| Issue | #134 |
| Branch | `wp/CON-09` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-con-09-2000, lease until 2026-11-03T20:33:15Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 7, 9, 10, 14, and 22 to the test configuration and module manifest schemas. The additions are a seed of at least 1, the port location rule, an optional where filter on criteria, module settings described by a JSON schema in each manifest, and an expected_bytes estimate per scenario.

## Plan

- [ ] 1. Test configuration schema changes for seed, port location, where, and module settings
- [ ] 2. Module manifest schema changes for settings_schema and expected_bytes
- [ ] 3. New examples for each addition and the change entry
- [ ] 4. Embedded controller copy of the schema, if CI compares it
- [ ] 5. Wiki ingest, checks, and pull request review

## Done and verified

None yet.

## Remaining

Every plan step.

## Next action

Edit contracts/schemas/test-config.schema.json for step 1.

## Blockers and open questions

The issue carries needs:owner, so the owner approves the contract change.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
make test-con
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T19:04 | claude-con-09-2000 | claim | none | Claimed from status ready |
