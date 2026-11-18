# Handoff AGT-28 The agent runs IPv6 host pools on the datapath

| Field | Value |
|---|---|
| Issue | #675 |
| Branch | `wp/AGT-28` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-a28c5e, lease until 2026-11-18T19:08:36Z |
| Last verified commit | none |

## Goal

The agent refuses a host pool without an IPv4 prefix on AF_XDP (`engine/fps-agent/src/backend.rs`, "IPv6 is TRN-30"). Pass IPv6 pools to the datapath (`Pools::pair_v6`, `sized_v6`) with their gateways for the neighbor resolution of TRN-35, so a forced AF_XDP run of the http1 and dns modules through the agent over IPv6 pools completes on a veth pair in CI.

## Plan

- [ ] 1. Read the pool code in `backend.rs` and the AF_XDP tests; claim commit.
- [ ] 2. Wire IPv6 pools into `backend.rs` with unit tests on fake hosts.
- [ ] 3. Extend `tests/afxdp_veth.rs` (or add a test) for an IPv6 run of http1 and dns.
- [ ] 4. Checks, log entry, Agent page, PR, CI.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read `engine/fps-agent/src/backend.rs` around line 424.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-agent --lib backend::
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T18:38Z | claude-a28c5e | claim | none | lease taken on #675 |
