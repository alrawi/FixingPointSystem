# Handoff CTL-51 Clock offsets of agents from heartbeats

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #595 |
| Branch | `wp/CTL-51` |
| Pull request | #0 |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | see the status card on #595 |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent manager records agent_time minus receive time per agent, `multiagent.HeartbeatClocks` exposes it as a ClockSource, fakeagent gains `ClockSkew`, and a test runs the CTL-16 check over real heartbeats: skew 15 ms refuses, 2 ms starts.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Manager records the offset, fakeagent skew option.
- [x] 2. HeartbeatClocks adapter and tests.
- [x] 3. Docs, log, handoff.
- [ ] 4. PR and CI.

## Done and verified

`gofmt -l controller` is clean. Go cannot build here, so CI runs the tests.

## Remaining

PR, CI green, review.

## Next action

Open the PR and read the controller `test` job.

## Blockers and open questions

none

## Decisions taken in this package

The adapter is in `multiagent` (it imports `agents`). The estimate includes one way heartbeat delay. Skew defaults to 0 for cli/e2e and UI-07.

## Pitfalls

none yet

## Verify the current state

```bash
gofmt -l controller   # prints nothing
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
