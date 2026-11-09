# Handoff ENG-19 Capture TCP sessions in the runtime

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #261 |
| Branch | `wp/ENG-19` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of N plan steps) |
| Current holder | claude-4e19a7, lease until 2026-11-09T05:22:58Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Move the capture cell into fps-capture, let fps-runtime read it when a client or server TCP session opens and wrap the session with Capture::tap (one atomic load when no capture runs), and expose the cell from the runtime. Acceptance: a runtime test starts a capture mid run and finds the sampled TCP sessions in the ring; tshark parses them where installed.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. CaptureCell in fps-capture with unit tests
- [ ] 2. fps-runtime tap opener, config and handle accessor
- [ ] 3. Runtime capture test (ring reader and tshark)
- [ ] 4. Docs, log entry, checks, PR

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Step 1: add engine/fps-capture/src/cell.rs.

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
