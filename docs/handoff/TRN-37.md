# Handoff TRN-37 DHCP leases on the datapath

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #720 |
| Branch | `wp/TRN-37` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 7 plan steps) |
| Current holder | claude-00af87, lease until 2026-11-28T07:39:53Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Plug `fps_hosts::DhcpHosts` into the dhcp hook of the arm whose pool leases, with a lease phase the caller waits on. The own stacks, the ARP responder, and the route of the arm read the lease table and follow its generation. Issue #720, bug #708.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Lease board in fps-tcp-own (shared lease table with generation), Station and Pools follow it
- [ ] 2. Stack reads leased addresses (connect by index, next pair)
- [ ] 3. DHCP hook, lease phase, DatapathConfig in fps-datapath
- [ ] 4. Splitter responder and Route follow the lease generation
- [ ] 5. In memory tests (no root), timeout error test
- [ ] 6. Harness test and CI
- [ ] 7. Docs, log entry, PR, review

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Write exactly one concrete step that the next agent can start immediately.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

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
| 2026-11-28T06:10 | claude-00af87 | claim | 502f726 | claimed #720 |
