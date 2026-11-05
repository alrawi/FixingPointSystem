# Handoff AGT-06 Kernel requirement and redial policy

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #138 |
| Branch | `wp/AGT-06` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-d5d4e4, lease until 2026-11-05T20:20:39Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 3 and 11. The agent refuses to start on Linux older than 6.3 with an error that names IP_LOCAL_PORT_RANGE, and it redials the controller with a backoff that restarts at 200 ms with jitter after an abort, and with a limit on failed redialing that defaults to five minutes.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Kernel version check through SystemRoot, with tests
- [ ] 2. Backoff reset after abort, with jitter, with tests
- [ ] 3. Redial limit flag and nonzero exit, with tests
- [ ] 4. Make targets, wiki Agent page, crate README, log entry, checks

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Step 1: read `src/capabilities.rs` `detect_kernel` and add the startup check.

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
| 2026-11-05T19:50Z | claude-d5d4e4 | claim | none | Claimed issue #138, card 6666407169 |
