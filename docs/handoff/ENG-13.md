# Handoff ENG-13 Run time rate adjustment

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #158 |
| Branch | `wp/ENG-13` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (4 of 4 plan steps) |
| Current holder | claude-7a2195, lease until 2026-11-07T18:53:08Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Let the scheduler of `engine/fps-sched/` change per scenario rates during a run without restarting workers. A thread safe handle takes effect at the next scheduling tick, the arrival process stays correct across the change, and rate 0 pauses.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Rate handle and rebase of the arrival process in `Schedule`
- [x] 2. `RateControl` registry with all or nothing updates
- [x] 3. Tests (tick, realized rate on a simulated clock, pause, unknown id, threads)
- [x] 4. Wiki page, log entry, checks, pull request

## Done and verified

- Steps 1 to 3: `cargo test -p fps-sched --offline` passes (20 tests in `tests/rate.rs`, 16 in the older files). `cargo clippy -p fps-sched --all-targets -- -D warnings` and `cargo fmt -p fps-sched -- --check` pass. `cargo check -p fps-runtime -p fps-agent` passes, so the additive `SchedError` variants and the `Schedule` fields break no consumer.
- Step 4: the Engine wiki page and the log entry are written. The repository checks and the pull request follow.

## Remaining

Open the pull request, wait for CI, and set the handoff to ready for review.

## Next action

Run `fps_swarm.py review 158 --pr <PR>` once CI is green.

## Blockers and open questions

none

## Decisions taken in this package

- The handle is one `AtomicU64` that holds the `f64` bits, and the schedule compares the bits with the ones it applied. No version counter is needed, a single word cannot tear, and a change from A to B and back to A between two ticks is correctly no change.
- The arrival process keeps the operational time of the pending arrival and rebases `op_base` and `mass_base` at the tick that applies the change. This keeps the expected count continuous, so there is no catch up burst and no gap, and the seeded sequence of an unchanged schedule stays bit identical.
- An arrival that is already due at the tick goes out under the old rate and the change waits for the first tick with none due. The alternative, applying at once, loses or misplaces the owed arrivals.
- `RateControl::update` takes a total per scenario for the whole agent (CON-05) and divides it in proportion to the current worker targets. `scale` takes the factor of CTL-21 against the registered target, so it is idempotent.
- Rate 0 pauses, and a concurrency target of 0 starts nothing. The constructor still refuses a zero target, as before.

## Pitfalls

A worker that sleeps with no due time (paused or at the cap) never sees a change, so the runtime must bound its sleep, see the log entry. The runtime is outside the paths of ENG-13.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-ENG-13 cargo test -p fps-sched --offline   # all tests pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-07 | claude-7a2195 | claim | none | Card 6710609923 |
