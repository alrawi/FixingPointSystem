# Handoff AGT-26 The agent resolves next hops on the datapath

| Field | Value |
|---|---|
| Issue | #671 |
| Branch | `wp/AGT-26` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-708cb1, lease until 2026-11-18T18:09:23Z |
| Last verified commit | none |

## Goal

The agent turns the neighbor resolution of the datapath (TRN-35) on when a host pool of the plan carries a gateway, and sets `OwnConfig::resolve_next_hop` for the same sides. Acceptance: a forced AF_XDP run through the agent reaches a peer behind a namespace router with no neighbor entry configured, in CI.

## Plan

- [ ] 1. Wiring in `backend.rs` (`datapath_config`) with unit tests on fake hosts.
- [ ] 2. Root test `tests/afxdp_router.rs`: the agent run over two namespaces and a kernel router.
- [ ] 3. Docs: this file, the log entry, `Agent.md`.
- [ ] 4. Checks, PR, CI.

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Build and run the unit tests of `backend::`.

## Blockers and open questions

none

## Decisions taken in this package

- One switch for both arms: `DatapathConfig::neighbors` is on when either host pool has a gateway. The datapath asks for the gateway of an arm whose pool has one and for the destination itself on an arm whose pool has none (TRN-35, `Route::of`).

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-agent --lib backend::
cargo test -p fps-agent --test afxdp_router -- --nocapture   # root, ip, sysctl
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T17:40Z | claude-708cb1 | claim | none | claimed #671 |
