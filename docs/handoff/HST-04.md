# Handoff HST-04 DHCP for user space backends

| Field | Value |
|---|---|
| Issue | #574 |
| Branch | `wp/HST-04` |
| Pull request | none yet |
| Status | in progress |
| Progress | 50% (3 of 6 plan steps) |
| Current holder | claude-5f0493, lease until 2026-11-17T04:58:41Z |
| Last verified commit | none |

## Goal

Hosts of a pool whose slice has `dhcp4` (CON-34) obtain and renew IPv4 addresses through DHCP, one client per host with MAC `mac_base + i`. The fps-hosts API gives the agent the leased addresses of the pool, so the agent can build the pool at Prepare and filter a DHCP port by those addresses.

## Plan

- [x] 1. DHCPv4 message codec (`dhcp/msg.rs`) with unit tests.
- [x] 2. Lease state machine (`dhcp/lease.rs`, renew at T1, rebind at T2, expiry) with unit tests.
- [x] 3. DHCP pool constructor and lease table (`DhcpPool`, `LeaseTable`) and the address source seen by `neighbor.rs` and `slice.rs`.
- [x] 4. Client driver over `FrameIo` plus the in test server.
- [ ] 5. Namespace harness test (root) and contract example test.
- [ ] 6. Wiki page, log entry, checks, PR, CI.

## Done and verified

- Steps 1 to 4: `cargo test -p fps-hosts` passes (112 unit tests, `tests/dhcp_port.rs` 13 tests over the in memory frame pair, no root), `cargo clippy -p fps-hosts --all-targets -- -D warnings` is clean.

## Remaining

Step 5 (namespace harness test with a server helper process and AF_XDP on the client arm), step 6 (wiki page, log entry, PR, CI).

## Next action

Write `tests/dhcp_harness.rs`: re-exec through `sudo -n`, QA-01 harness, server helper in the router namespace, client helper in the client namespace.

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
