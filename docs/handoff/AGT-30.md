# Handoff AGT-30 Datapath lanes follow the workers of the plan

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #680 |
| Branch | `wp/AGT-30` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-a30e71, lease until 2026-11-27T23:54:44Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Start one datapath lane per plan worker, up to the queue count of each port, give each lane the hosts of its workers, and report the lanes, so an AF_XDP run scales on a multi queue port.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Lane planning from the plan workers, queue count of a port, and fake host tests
- [ ] 2. Wire into start_af_xdp and the run, report lanes
- [ ] 3. Veth pair with four queues test, docs, log entry, checks

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
| 2026-11-27 | claude-a30e71 | claim | - | claimed #680 |
