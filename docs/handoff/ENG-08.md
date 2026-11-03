# Handoff ENG-08 Session scheduler

| Field | Value |
|---|---|
| Issue | #28 |
| Branch | `wp/ENG-08` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-eng-08-da83, lease until 2026-11-03T04:13:22Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Produce session start times from the load objective, the load phases, and the arrival process, deterministically from the seed. The crate `engine/fps-sched` offers constant and Poisson arrivals, a concurrency cap, hold, linear, and step phases, and a schedule iterator that a worker drains with an injected clock.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Crate skeleton, phase model, and the fraction and cumulative load functions
- [ ] 2. Rate driven schedule with constant and Poisson arrivals and the concurrency cap
- [ ] 3. Concurrency driven schedule and the clock driven drain iterator
- [ ] 4. Tests for determinism, the KS test, and the phase boundaries, with property tests
- [ ] 5. Engine wiki section, log entry, and pull request ready for review

## Done and verified

None yet.

## Remaining

Every plan step.

## Next action

Create `engine/fps-sched` with the phase model.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cargo test -p fps-sched
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-eng-08-da83 | claim | none | Claimed with card 6594093107 |
