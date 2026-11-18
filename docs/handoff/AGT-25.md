# Handoff AGT-25 The agent runs its workers on the datapath

| Field | Value |
|---|---|
| Issue | #662 |
| Branch | `wp/AGT-25` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-eda452, lease until 2026-11-18T15:50:20Z |
| Last verified commit | none |

## Goal

`Prepared::start_on` passes the datapath of the AF_XDP backend to the workers through the `Transport` of ENG-31 and drops the `Unimplemented` that AGT-24 left. The agent turns the `os_profile` name of each host pool into `OsProfile::builtin` for the own stack. Acceptance: a forced AF_XDP run of the http1 and dns modules through the agent completes on a veth pair in CI.

## Plan

- [ ] 1. A `Transport` over the datapath (`src/transport.rs`): listen once per port, route accepted sessions by `local_addr`, TCP and UDP.
- [ ] 2. The agent wiring: the UDP workers of `udp.rs` open sessions through a `Transport`, `start_on` hands the datapath transport to every worker, `Active` keeps the backend and stops it, the os profiles reach `OwnConfig`.
- [ ] 3. Tests on fake hosts, and `tests/afxdp_veth.rs` runs the http1 and dns entries through the agent.
- [ ] 4. Docs: this file, the log entry, `Agent.md`.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write `engine/fps-agent/src/transport.rs`.

## Blockers and open questions

none

## Decisions taken in this package

- The datapath transport owns one background Tokio runtime (the hub), because `Datapath::listen` spawns its accept tasks on the runtime of the caller and a worker runtime ends with its worker. The hub keeps one datapath listener per port for the whole run and routes each accepted session to the worker whose `listen` call named its `local_addr()`.

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-AGT-25 cargo test -p fps-agent --lib
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T14:20Z | claude-eda452 | claim | none | claimed #662 |
