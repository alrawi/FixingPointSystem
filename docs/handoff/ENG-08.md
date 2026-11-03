# Handoff ENG-08 Session scheduler

| Field | Value |
|---|---|
| Issue | #28 |
| Branch | `wp/ENG-08` |
| Pull request | #86 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-eng-08-da83, lease until 2026-11-03T04:13:22Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Produce session start times from the load objective, the load phases, and the arrival process, deterministically from the seed. The crate `engine/fps-sched` offers constant and Poisson arrivals, a concurrency cap, hold, linear, and step phases, and a schedule iterator that a worker drains with an injected clock.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Crate skeleton, phase model, and the fraction and cumulative load functions
- [x] 2. Rate driven schedule with constant and Poisson arrivals and the concurrency cap
- [x] 3. Concurrency driven schedule and the clock driven drain iterator
- [x] 4. Tests for determinism, the KS test, and the phase boundaries, with property tests
- [x] 5. Engine wiki section, log entry, and pull request ready for review

## Done and verified

1. Steps 1 to 4 are verified by `cargo test -p fps-sched`, which passes 25 tests. The unit tests in `src/profile.rs` cover fractions, the integral and its inverse, step levels, and validation. `tests/schedule.rs` covers determinism, tick independence, the `next_due` worker loop, the KS tests, the phase boundaries, the concurrency targets, and both cap policies. `tests/properties.rs` holds the property tests and `tests/contract.rs` the contract tests.
2. `cargo clippy -p fps-sched --all-targets -- -D warnings` and `cargo fmt --all -- --check` pass.
3. `make lint test` passes. Under heavy host load the spike test `interp::tests::pattern_split_across_reads` of `test-spk` once hung, and `make test-spk` alone passed, so we treat it as load related and outside this package.
4. Step 5 added the scheduler section at the end of `docs/wiki/Engine.md` and the log entry `docs/wiki/log/2026-11-03T0304-ingest-eng-08-session-scheduler.md`.

## Remaining

Nothing for this package. The throughput and simulated user objectives reach the scheduler as rate and concurrency targets, because the plan compiler maps them as `contracts/README.md` states. Live rate updates belong to a later package that depends on ENG-08.

## Next action

Review the pull request.

## Blockers and open questions

none

## Decisions taken in this package

1. Rate starts use the time change method, so the expected count of every phase is exact and Poisson arrivals form a nonhomogeneous Poisson process.
2. Constant start `k` lies where the expected count reaches `k + 0.5`, which keeps counts exact at phase boundaries without floating point ties.
3. Step `i` of `n` holds `start + (end - start) * i / n`, following the proto comment that `start_fraction` holds at the phase start.
4. The cap defers rate arrivals by default and keeps their scheduled time. `CapPolicy::Skip` drops them and counts them.
5. Concurrency starts carry the drain time, since they depend on when sessions close.
6. The generator state keeps the stream above the two lowest bits, which `Pcg64Mcg` forces to 1, and mixes the seed into the high half with SplitMix64.
7. No new dependency. `rand`, `rand_pcg`, `thiserror`, `proptest`, and `serde_json` were already in `Cargo.lock`, which only gains the `fps-sched` entry.

## Pitfalls

1. `Pcg64Mcg::new(seed << 64 | stream)` gives streams 0 to 3 the same sequence. The test `streams_of_one_seed_differ` caught it, and the log entry asks the coordinator to fix the `session_rng` example in Coding Standards.
2. golangci-lint refuses to run beside another agent's copy unless `TMPDIR` points at a private directory.

## Verify the current state

```bash
export CARGO_TARGET_DIR=/home/claude/agents/scratch-eng-08/target
cargo test -p fps-sched        # 25 tests pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-eng-08-da83 | claim | none | Claimed with card 6594093107 |
| 2026-11-03T03:10Z | claude-eng-08-da83 | finish | see branch | Ready for review |
