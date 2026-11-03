# Handoff HST-01 Host pool allocator

| Field | Value |
|---|---|
| Issue | #34 |
| Branch | `wp/HST-01` |
| Pull request | #83 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-hst-01-477c, lease until 2026-11-03T04:22:35Z |
| Last verified commit | see the finish row of the session log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We map sessions to source hosts deterministically and split host pools across agents and workers. The crate `engine/fps-hosts` offers round robin, uniform, and Zipf host selection from the run seed, and slicing of a pool by agent and by worker without overlap for IPv4 and IPv6 prefixes.

## Plan

- [x] 1. Crate skeleton, pool model, and host index to address mapping
- [x] 2. Slicing by agent and by worker with property tests
- [x] 3. Round robin, uniform, and Zipf selection with tests
- [x] 4. Hosts wiki page, log entry, and full checks

## Done and verified

- Steps 1 to 3 are verified by `cargo test -p fps-hosts`, with 23 unit and property tests and 3 contract tests passing, and by `cargo clippy -p fps-hosts --all-targets -- -D warnings`.
- Step 4 is verified by `make lint test`, which includes `tools/plan/plan.py lint` for the new Hosts page.

## Remaining

Nothing in scope. Known limits follow.

- Zipf picks use `ln` and `exp` from the platform math library, so a replay on another platform may differ in rare picks.
- Zipf rank 1 is the first host of the worker range. A later package may add a seeded permutation if hot hosts should spread over the prefix.
- Selection modes are an API of the crate only. Neither the test configuration schema nor the agent contract names a selection mode yet, so a CON package must add the field before a plan can choose one.

## Next action

Review pull request #83.

## Blockers and open questions

none

## Decisions taken in this package

- A prefix with nonzero host bits is an error, because the contract names the network address of the pool.
- A gateway skips an address only when it lies strictly inside the usable part of the prefix.
- `HostRange::split` gives each positive weight one host first, then splits the rest by largest remainder with ties to the lower part. This keeps every worker with a positive weight busy when the pool is large enough.
- The session generator puts a mixed session id in the high half of the `Pcg64Mcg` state and a mixed value in the low half. `Pcg64Mcg::new` sets the two lowest bits, and a constant low half biased the first outputs, which a uniform histogram showed. Bug #90 tracks the same collision in the Coding Standards example.
- Round robin maps session id `s` to `offset + s mod count` without state, so a replay needs no counter.
- Contract tests parse the plan JSON with `serde_json` and the YAML with `serde_norway`, both dev dependencies already in `Cargo.lock`.

## Pitfalls

- `Pcg64Mcg::new((seed << 64) | session_id)` gives ids 4k to 4k+3 the same stream. See bug #90.
- The proto3 JSON mapping omits zero fields, so `workerId` and `offset` may be missing in plan JSON.

## Verify the current state

```bash
cargo test -p fps-hosts
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-hst-01-477c | claim | none | Card 6594101938 |
| 2026-11-03T03:03Z | claude-hst-01-477c | finish | see git log | make lint test passed; ready for review |
