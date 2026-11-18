# Handoff TRN-12 User space TCP over AF_XDP, end to end

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #563 |
| Branch | `wp/TRN-12` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-7c41e9, lease until 2026-11-18T17:31:32Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The http1 module passes realism CI on the AF_XDP backend: the agent forced onto AF_XDP runs the http1 scenarios over a veth pair in the namespace harness and the capture scores like the kernel run.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Read the code, run afxdp_veth locally
- [ ] 2. Script that records the http1 run on AF_XDP
- [ ] 3. Fix what the run finds in the paths
- [ ] 4. Log entry, component page, checks
- [ ] 5. PR and CI

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
