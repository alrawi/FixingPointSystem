# Handoff ENG-10 Worker runtime

| Field | Value |
|---|---|
| Issue | #30 |
| Branch | `wp/ENG-10` |
| Pull request | #103 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-eng-10-c6ab, lease until 2026-11-03T14:25:52Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Run one single threaded Tokio runtime per worker thread, with optional core pinning. Each worker owns its sessions, timers, and statistics shard. The runtime supports a graceful stop with ramp down and an immediate abort.

## Plan

- [x] 1. Crate skeleton, worker configuration, and thread spawning with optional pinning
- [x] 2. Tokio backed clock for fps-sched and the client session loop
- [x] 3. Server workers over kernel listeners
- [x] 4. Statistics shards and the one second snapshot to a collector
- [x] 5. Graceful stop with ramp down and immediate abort, with tests
- [x] 6. Loopback acceptance test, wiki section, and log entry

## Done and verified

- Steps 1 to 5. `cargo test -p fps-runtime` passes 6 unit tests, 7 loopback tests, and 2 property tests, five runs in a row without a failure.
- Step 6. `tests/loopback.rs` drives the http1 `web_get` example at 50 sessions per second per client worker for two seconds with two client and two server workers. Both sides report 200 attempted and 200 succeeded sessions and no failure, and the byte counters of the two sides match. The stop tests assert that `join` returns within the ramp down time plus one second.
- `cargo clippy -p fps-runtime --all-targets -- -D warnings` and `cargo fmt --check` pass.

## Remaining

- The runtime records no `sessions.response_time`, `transactions.*`, `actions.*`, or `tcp.*` series yet. The interpreter exposes only byte counters and the failure per session.
- No performance benchmark of the runtime exists yet. The bench workstream can add one on top of `Runtime::start`.
- Building a `RuntimeConfig` from the agent plan belongs to AGT-01.

## Next action

Review the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- One `LocalSet` per worker thread, so session tasks use `Rc` and `RefCell` and the shard needs no lock.
- `core_affinity` for pinning, so the crate keeps `forbid(unsafe_code)`. A failed pin is reported in `WorkerExit::pinned` and does not stop the worker.
- The worker id fills the top 16 bits of every session id, see `global_session_id`.
- The collector merges client and server workers separately, because the catalog session metrics have no `side` label.
- A ramp down deadline starts when a worker sees the stop. Sessions open at the deadline are aborted through the `stop` future of `fps-interp`, and count as `aborted`.
- The server opener hands the interpreter an already accepted stream, so one accept loop per listener serves every session.

## Pitfalls

- `tokio::sync::watch::Sender::send` fails without receivers and then does not store the value, so the abort flag uses `send_replace`.

## Verify the current state

```bash
cargo test -p fps-runtime
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T12:56Z | claude-eng-10-c6ab | claim | none | fresh claim |
| 2026-11-03T13:15Z | claude-eng-10-c6ab | finish | see PR | ready for review |
