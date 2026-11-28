# Handoff CON-36 The DHCP example names a port for each arm

| Field | Value |
|---|---|
| Issue | #721 |
| Branch | `wp/CON-36` |
| Pull request | see issue |
| Status | ready for review |
| Progress | 100% (2 of 2 plan steps) |
| Current holder | claude-con36a |
| Last verified commit | see git log |

## Goal

Give each arm of `contracts/examples/tests/dhcp_clients.yaml` its own interface and the server pool a gateway, so the file describes clients that lease from the device under test (bug #708).

## Plan

- [x] 1. Example, header comment, tests of `tools/contracts/test_contracts.py`
- [x] 2. `contracts/README.md`, `contracts/changes/CON-36.md`, log entry, handoff

## Done and verified

`python3 tools/contracts/validate.py` (0 errors), `python3 -m pytest -q tools/contracts` (345 passed, 2 skipped), and `plan.py check`, `render --check`, `lint` pass.

## Remaining

The coordinator applies the controller golden (see the Shared page notes of the log entry). CI checks the Go and agent tests.

## Next action

Coordinator: apply the golden edits to the branch, then merge when CI is green.

## Blockers and open questions

none

## Decisions taken in this package

See the log entry `2026-11-28T0730-ingest-con-36-the-dhcp-example-names-a-port-for-each-arm.md`.

## Pitfalls

The compiler refuses a pinned port on an agent whose interface list lacks it, so the golden case must list `eth1` and `eth2`.

## Verify the current state

```bash
python3 tools/contracts/validate.py && python3 -m pytest -q tools/contracts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-28T07:22Z | claude-con36a | claim and finish | see git log | one session |
