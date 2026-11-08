# Handoff ENG-18 Rate handles and paused worker wake in the runtime

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #240 |
| Branch | `wp/ENG-18` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-9bcc05, lease until 2026-11-08T22:22:24Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The runtime returns a `fps_sched::RateControl` that maps each mix entry name to the rate handles of its workers, and a client worker with nothing due wakes at least every 100 ms to read its handle.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Expose handles: workers build their schedule before ready and report the handle; the runtime registers them.
- [x] 2. Bounded wake in the client loop.
- [x] 3. Runtime test: pause, resume, two rate changes.
- [ ] 4. Wiki, log entry, checks, PR.

## Done and verified

`cargo test -p fps-runtime` green (unit, loopback incl. 2 new tests, properties); `cargo clippy -p fps-runtime --all-targets -- -D warnings` and `cargo fmt --check` clean; `cargo check -p fps-agent --all-targets` builds. The paused wake test fails with RATE_POLL set to 10 s.

## Remaining

Step 4: PR and CI. Gap: UDP workers of fps-agent register no handles.

## Next action

Open the PR and wait for CI.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

`FlowRef::entry` (optional) names the mix entry, because ClientConfig is built by literal in fps-agent, which this package may not touch. The poll caps every client sleep, not only the None case, so a low rate raised while a distant start is pending also reacts within 100 ms. The Schedule is built before ready so the handle is registered before start returns.

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
| 2026-11-08 | claude-9bcc05 | claim | - | claimed #240 |
