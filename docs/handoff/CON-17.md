# Handoff CON-17 The tls_upgrade action in the scenario contract

| Field | Value |
|---|---|
| Issue | #322 |
| Branch | `wp/CON-17` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (3 of 3 plan steps, PR and CI pending) |
| Current holder | claude-c17a4e |
| Last verified commit | none |

## Goal

Add the `tls_upgrade` flow action to the scenario contract (schema keyword, README row, validator rule that it is an error in a UDP flow, change entry, version bump, changelog). The owner approved the change.

## Plan

- [x] 1. Schema keyword, embedded Go copy, README row, validator rule
- [x] 2. Tests in tools/contracts, change entry, version bump, CHANGELOG, Contracts wiki page, log entry
- [x] 3. Checks, pull request, CI

## Done and verified

Schema keyword, Go embedded copy, README row, validator rule, tests, CON-17 entry, v0.4 bump, CHANGELOG, Contracts page, log entry. `make test-con` passes (185 passed, 1 skipped); plan.py check, render --check, lint pass.

## Remaining

CI result. fps-flow check_action for UDP is an ENG change (reported).

## Next action

Poll CI, then run fps_swarm.py review.

## Blockers and open questions

none

## Decisions taken in this package

Version v0.3 to v0.4 (minor), because the change adds a keyword, as CON-04 and CON-10 did. The Go embedded schema copy is refreshed because its test compares it with the contract.

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
