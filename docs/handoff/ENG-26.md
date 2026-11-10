# Handoff ENG-26 Datagram flows in multi flow instances and data ports without repeats

| Field | Value |
|---|---|
| Issue | #422 |
| Branch | `wp/ENG-26` |
| Pull request | none yet |
| Status | in progress |
| Progress | 80% (5 of 6 plan steps) |
| Current holder | claude-d50d53, lease until 2026-11-11T00:05:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Give the instance runner of `engine/fps-runtime` a datagram opener (bound server socket per port shared by its sessions, a socket per media port bound for the flow, a connected client socket with its own source port per flow), and stop two concurrent instances that draw the same `rand_int` data port from colliding on one listener (AGT-18). Do not edit `engine/fps-agent/` (AGT-20) or `modules/`.

## Plan

- [x] 1. Read the runtime, fixture_peer.rs, and the MOD-14 and AGT-18 log entries
- [x] 2. Datagram opener in InstanceEnv
- [x] 3. Data port collision fix
- [x] 4. Runtime tests: SIP call with RTP through the instance runner, two instances on one drawn port
- [x] 5. Checks, log entry, Engine page
- [ ] 6. PR and CI

## Done and verified

- `cargo test -p fps-runtime` passes (22 unit tests, 2 datagram tests, 7 instance tests, the older suites).
- `cargo clippy -p fps-runtime --all-targets -- -D warnings` and `cargo fmt --all -- --check` are clean.
- `concurrent_instances_that_draw_the_same_data_port_all_succeed` failed in 2 of 3 runs on the runtime before this package (27 or 28 of 32 sessions) and passes 5 of 5 runs after it.
- `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Open the PR, wait for CI, `fps_swarm.py review`. `cargo check -p fps-agent --tests` was started to show that the agent still builds against the runtime.

## Next action

Open the pull request and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- Data ports: the listener serves every waiting instance and matches each connection to the instance that expects its source host. A change to the draws is impossible inside `fps-runtime`, because `rand_int` is a core function of `fps-flow` and the client renders the same value from its own generator.
- UDP server sides use Tokio sockets with a task per port that sorts datagrams by source address, not `KernelDatagramListener`, whose sockets all set `SO_REUSEPORT` and would split a port between two instances.
- UDP client sides use `Connector::connect_datagram`, which honours the local port range of the connect configuration.

## Pitfalls

- The worktree guard refuses `git` in a compound command or after `.`. Run each git command alone.
- A fresh `cargo test -p fps-runtime --no-run` compiles wasmtime and takes about ten minutes on one job.

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=1 cargo test -p fps-runtime
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T22:36Z | claude-d50d53 | claim | none | Claimed #422 with card 6781155090 |
| 2026-11-10T23:10Z | claude-d50d53 | checkpoint | see branch | Datagram opener, shared data ports, tests, Engine page, log entry |
