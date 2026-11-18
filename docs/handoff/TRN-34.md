# Handoff TRN-34 Pacing and faster loss detection in the own stack

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #659 |
| Branch | `wp/TRN-34` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-50b39c, lease until 2026-11-18T19:37:47Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add sender pacing, HyStart for the CUBIC slow start, and tail loss probes with RACK to the own TCP stack, each behind a switch in `OwnConfig`. In the bottleneck test the own stack retransmits at most twice as many segments as the kernel at no lower goodput, and the TRN-22 to TRN-25 acceptance rows still pass.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Read the recovery, CUBIC and host code; baseline of the bottleneck test
- [ ] 2. HyStart in `cubic.rs`
- [ ] 3. Pacing in `host.rs`
- [ ] 4. RACK and tail loss probe in `recovery.rs`
- [ ] 5. Defaults, tests, bench comparison, Transport.md
- [ ] 6. Checks, PR, CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Step 1.

## Blockers and open questions

none

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
| 2026-11-18T18:08Z | claude-50b39c | claim | none | claimed #659 |
