# Handoff CON-10 Behavior profiles and distributions in the scenario contract

| Field | Value |
|---|---|
| Issue | #152 |
| Branch | `wp/CON-10` |
| Pull request | #222 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-con10-a1b2c3, lease until 2026-11-06T23:01:56Z |
| Last verified commit | 9d0a3b8 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 item 18. Define the `profiles` root and the `dist` functions in the scenario grammar so that the specification example validates, and record the change in `contracts/changes/CON-10.md`.

## Plan

- [x] 1. Review the WIP snapshot of the stopped worker and complete the missing example
- [x] 2. Change note, wiki section, and log entry
- [x] 3. Local checks
- [x] 4. Pull request and CI

## Done and verified

- Step 1. The snapshot lacked `scenarios-pending/web_profile.yaml`, which its tests read. I wrote it, restored `contracts/README.md` to main (outside the package, so the grammar text sits in the change note for the coordinator), and updated the module digest in the example plan and the compiler goldens.
- Step 4. CI on 9d0a3b8 is green. The UI test `Modules.test.tsx` asserted the old `http1.request` signature, so I updated its expected string.
- Step 3. `python3 -m pytest -q tools/contracts` passes (171 tests) and `python3 tools/contracts/validate.py` reports 0 errors.

## Remaining

The owner approval of the contract change.

## Next action

The coordinator reviews pull request #222.
