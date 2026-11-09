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
| Progress | 90% (3 of 4 plan steps, PR and CI pending) |
| Current holder | claude-4e19a7, lease until 2026-11-09T05:22:58Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Move the capture cell into fps-capture, let fps-runtime read it when a client or server TCP session opens and wrap the session with Capture::tap (one atomic load when no capture runs), and expose the cell from the runtime. Acceptance: a runtime test starts a capture mid run and finds the sampled TCP sessions in the ring; tshark parses them where installed.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. CaptureCell in fps-capture with unit tests
- [x] 2. fps-runtime tap opener, config and handle accessor
- [x] 3. Runtime capture test (ring reader and tshark)
- [x] 4. Docs, log entry, checks, PR

## Done and verified

`cargo test -p fps-capture -p fps-runtime` all green (3 new cell tests, 3 new runtime capture tests, loopback 9 green); `cargo clippy -p fps-runtime -p fps-capture --all-targets -D warnings` clean; `cargo check -p fps-agent --all-targets` builds. plan.py check, render --check, lint pass.

## Remaining

Open the PR and wait for CI. tshark did not run locally.

## Next action

Open the PR, wait for CI, then run fps_swarm.py review.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Tap in the opener (TapOpener), not in fps-interp, so the interpreter stays free of capture. The cell is read once per open and the socket is asked for addresses only while a capture runs. The agent's cell is left in place (AGT-08 works there).

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
