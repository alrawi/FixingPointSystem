# Handoff HST-04 DHCP for user space backends

| Field | Value |
|---|---|
| Issue | #574 |
| Branch | `wp/HST-04` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-5f0493, lease until 2026-11-17T04:58:41Z |
| Last verified commit | none |

## Goal

Hosts of a pool whose slice has `dhcp4` (CON-34) obtain and renew IPv4 addresses through DHCP, one client per host with MAC `mac_base + i`. The fps-hosts API gives the agent the leased addresses of the pool, so the agent can build the pool at Prepare and filter a DHCP port by those addresses.

## Plan

- [ ] 1. DHCPv4 message codec (`dhcp/msg.rs`) with unit tests.
- [ ] 2. Lease state machine (`dhcp/lease.rs`, renew at T1, rebind at T2, expiry) with unit tests.
- [ ] 3. DHCP pool constructor and lease table (`DhcpPool`, `LeaseTable`) and the address source seen by `neighbor.rs` and `slice.rs`.
- [ ] 4. Client driver over `FrameIo` plus the in test server.
- [ ] 5. Namespace harness test (root) and contract example test.
- [ ] 6. Wiki page, log entry, checks, PR, CI.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read `engine/fps-hosts/src/{pool,slice,neighbor,wire}.rs` and start step 1.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-HST-04 cargo test -p fps-hosts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-17T04:28Z | claude-5f0493 | claim | none | claimed #574 |
