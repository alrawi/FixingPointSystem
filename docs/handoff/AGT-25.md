# Handoff AGT-25 The agent runs its workers on the datapath

| Field | Value |
|---|---|
| Issue | #662 |
| Branch | `wp/AGT-25` |
| Pull request | #668 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-eda452, lease until 2026-11-18T15:50:20Z |
| Last verified commit | 3dfc75f |

## Goal

`Prepared::start_on` passes the datapath of the AF_XDP backend to the workers through the `Transport` of ENG-31 and drops the `Unimplemented` that AGT-24 left. The agent turns the `os_profile` name of each host pool into `OsProfile::builtin` for the own stack. Acceptance: a forced AF_XDP run of the http1 and dns modules through the agent completes on a veth pair in CI.

## Plan

- [x] 1. A `Transport` over the datapath (`src/transport.rs`): listen once per port, route accepted sessions by `local_addr`, TCP and UDP.
- [x] 2. The agent wiring: the UDP workers of `udp.rs` open sessions through a `Transport`, `start_on` hands the datapath transport to every worker, `Active` keeps the backend and stops it, the os profiles reach `OwnConfig`.
- [x] 3. Tests on fake hosts, and `tests/afxdp_veth.rs` runs the http1 and dns entries through the agent.
- [x] 4. Docs: this file, the log entry, `Agent.md`.

## Done and verified

- `cargo test -p fps-agent` passes (the whole crate, including `afxdp_veth` as root in this sandbox: 87 client and 87 server sessions, 0 failed, verdict pass, 1900 TCP frames and 48 UDP datagrams through the queues).
- `cargo clippy -p fps-agent --all-targets -- -D warnings` and `cargo fmt -p fps-agent --check` are clean.
- `plan.py check`, `render --check`, and `lint` pass.
- CI of PR #668 on 3dfc75f: every check green, including `rust` and the `afxdp` job of `trn.yml`, which runs `afxdp_veth` with `CI=true`.

## Remaining

Nothing. The coordinator reviews and merges PR #668.

## Next action

Review PR #668.

## Blockers and open questions

none

## Decisions taken in this package

- The datapath transport owns one background Tokio runtime (the hub), because `Datapath::listen` spawns its accept tasks on the runtime of the caller and a worker runtime ends with its worker. The hub keeps one datapath listener per port for the whole run and routes each accepted session to the worker whose `listen` call named its `local_addr()`.
- A port that a worker asked for by number stays open for the run (the data ports bind and release often, and a port cannot be listened on twice at once). A port that the transport chose for a listen on port 0 closes with its last call.
- The UDP workers keep their kernel sockets when no transport is set, so the kernel path is byte for byte as before. With a transport they listen once for each address of their range.
- `backend::start_on_queues` exists so the wiring is tested without root on in memory links.

## Pitfalls

- `EndGuard` must be created before the async block, or a task that the hub drops before its first poll never clears its port and `accept` waits for ever.
- The reset of a session for an unheld host can fail the client `connect` or its first read, so a test has to accept both.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-AGT-25 cargo test -p fps-agent --test datapath_transport --test datapath_run
sudo FPS_REQUIRE_AFXDP=1 CARGO_TARGET_DIR=$PWD/target-AGT-25 cargo test -p fps-agent --test afxdp_veth -- --nocapture
```

Both pass. The second prints `run outcome=pass sessions http1=.. dns=..`.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T14:20Z | claude-eda452 | claim | none | claimed #662 |
| 2026-11-18T14:45Z | claude-eda452 | checkpoint | 834eeb7 | transport, wiring, tests, docs |
| 2026-11-18T15:35Z | claude-eda452 | ready for review | 3dfc75f | PR #668, CI green |
