# Handoff CON-17 The tls_upgrade action in the scenario contract

| Field | Value |
|---|---|
| Issue | #322 |
| Branch | `wp/CON-17` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-c17a4e |
| Last verified commit | none |

## Goal

Add the `tls_upgrade` flow action to the scenario contract (schema keyword, README row, validator rule that it is an error in a UDP flow, change entry, version bump, changelog). The owner approved the change.

## Plan

- [ ] 1. Schema keyword, embedded Go copy, README row, validator rule
- [ ] 2. Tests in tools/contracts, change entry, version bump, CHANGELOG, Contracts wiki page, log entry
- [ ] 3. Checks, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Edit contracts/schemas/scenario.schema.json.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
python3 -m pytest tools/contracts -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T21:10Z | claude-c17a4e | claim | none | Claimed #322 |
