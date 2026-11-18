# Handoff AGT-26 The agent resolves next hops on the datapath

| Field | Value |
|---|---|
| Issue | #671 |
| Branch | `wp/AGT-26` |
| Pull request | #673 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-708cb1, lease until 2026-11-18T18:09:23Z |
| Last verified commit | aa20461 |

## Goal

The agent turns the neighbor resolution of the datapath (TRN-35) on when a host pool of the plan carries a gateway, and sets `OwnConfig::resolve_next_hop` for the same sides. Acceptance: a forced AF_XDP run through the agent reaches a peer behind a namespace router with no neighbor entry configured, in CI.

## Plan

- [x] 1. Wiring in `backend.rs` (`datapath_config`) with unit tests on fake hosts.
- [x] 2. Root test `tests/afxdp_router.rs`: the agent run over two namespaces and a kernel router.
- [x] 3. Docs: this file, the log entry, `Agent.md`.
- [x] 4. Checks, PR, CI.

## Done and verified

- `cargo test -p fps-agent --lib backend::` passes (12 tests, 2 new: the switch for no gateway, either pool, both; and a datapath on in memory links that asks for the gateway of the client arm).
- `cargo test -p fps-agent --test afxdp_router` passes as root in this sandbox (3.8 s): http1 63 and dns 24 sessions, none failed; each arm: 1 request, 1 resolution, 0 failed, only the gateway as an entry; the router learned the hosts of both pools.
- `cargo clippy -p fps-agent --all-targets -- -D warnings` and `cargo fmt -p fps-agent` are clean. `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Nothing more in the package. CI of PR #673 on aa20461: all 54 checks green, including rust and afxdp. Gaps that stay: DHCP pools (the datapath has no DHCP hook, so the agent still refuses them on AF_XDP and `LeaseTable::router_of` is not read), and the `afxdp` job of `trn.yml` needs the step in the log entry's shared page notes (a coordinator file).

## Next action

Review PR #673.

## Blockers and open questions

none

## Decisions taken in this package

- One switch for both arms: `DatapathConfig::neighbors` is on when either host pool has a gateway. The datapath asks for the gateway of an arm whose pool has one and for the destination itself on an arm whose pool has none (TRN-35, `Route::of`).

## Pitfalls

- The plan most likely passes without the change too (not run), because the pools hold the MAC address of every host and the frames never touch the router. The test asserts the neighbor counters and the router's neighbor table, not only the sessions.
- A plan run's peer is always an own stack host, so a kernel socket peer is not possible through the agent. The kernel is the router here.
- The first build of a fresh target dir takes about 6 minutes with one job.

## Verify the current state

```bash
cargo test -p fps-agent --lib backend::
cargo test -p fps-agent --test afxdp_router -- --nocapture   # root, ip, sysctl
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T17:40Z | claude-708cb1 | claim | none | claimed #671 |
| 2026-11-18T18:15Z | claude-708cb1 | finish | aa20461 | PR #673 opened, CI green, ready for review |
