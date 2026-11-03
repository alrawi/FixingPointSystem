# Handoff ENG-07 Multi flow scenario instances

| Field | Value |
|---|---|
| Issue | #27 |
| Branch | `wp/ENG-07` |
| Pull request | #120 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-eng-07-74e3, lease until 2026-11-03T16:39:34Z |
| Last verified commit | the head of `wp/ENG-07` at review |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add a scenario instance runner to `fps-interp` that owns shared scenario variables and flow states, runs flows in `after` order and through `spawn`, and fails the instance when a spawned flow fails.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim, branch, handoff, and pull request
- [x] 2. Read contract v0.2 rules for after, spawn, set, and failure propagation
- [x] 3. `ScenarioInstance` with flow states, ordering, spawn, and shared variables
- [x] 4. Tests for web_browse ordering and spawned flow failure
- [x] 5. Wiki ingest, log entry, final checks, and review

## Done and verified

| Step | Command | Result |
|---|---|---|
| 3, 4 | `cargo test -p fps-interp --test instances` | 7 tests pass, the two acceptance tests among them |
| 3 | `cargo test -p fps-flow` | 37 unit tests pass, `instance_mode_compiles_spawn_and_keeps_flow_relations` among them |
| 5 | `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check` | pass |
| 5 | `make lint test` | pass |

Acceptance criteria.

- The web_browse example runs its lookup flow before its page flow in `web_browse_runs_lookup_before_page`, which also checks that the page dials the address that the lookup bound with `set`.
- A spawned flow failure fails its parent instance in `spawned_flow_failure_fails_its_parent_and_the_instance`, where the error names the data flow spawned by the control flow and both flows end failed.

## Remaining

- The specification example of web_browse uses `transport: tls` and module calls that no module ships yet, so the test runs plain HTTP with test doubles of the `dns` functions.
- `fps-runtime` still runs flows one by one. Wiring `ScenarioInstance` into the workers belongs to a runtime package.

## Next action

Review pull request #120 against the Definition of Done.

## Blockers and open questions

- The contract names no rule for the other flows after a failure. We fail fast, and a CON package could write the rule into `contracts/README.md`.

## Decisions taken in this package

- `fps-flow` gained the additive `Scenario::parse_instance` and `load_instance`, `Op::Spawn`, and `Flow::after` and `Flow::spawned`, while `parse` and `load` keep rejecting `spawn`, so the one flow runner of `fps-runtime` sees no change.
- A `spawn` starts the same side of the target flow, and a spawned side reads its parent side variables at the `spawn`.
- The first failure aborts the rest of the instance, and a spawned failure fails the whole spawn chain.
- The abort signal is a `tokio::sync::watch` channel, so `fps-interp` enables the `sync` feature of Tokio. `Cargo.lock` did not change.

## Pitfalls

- CI runs a newer clippy than the first local run, and `result_large_err` rejected `InstanceError` at 136 bytes. The error now boxes its `Failure`, and a unit test pins the size at 128 bytes or less.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T15:09:35Z | claude-eng-07-74e3 | claim | | Card 6607414675 |
| 2026-11-03T15:24:07Z | claude-eng-07-74e3 | checkpoint | | Instance runner and tests |
| 2026-11-03T15:30:00Z | claude-eng-07-74e3 | finish | | Wiki ingest, ready for review |
| 2026-11-03T15:40:00Z | claude-eng-07-74e3 | checkpoint | | Box the failure of InstanceError for result_large_err |
