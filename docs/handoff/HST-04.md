# Handoff HST-04 DHCP for user space backends

| Field | Value |
|---|---|
| Issue | #574 |
| Branch | `wp/HST-04` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (5 of 6 plan steps) |
| Current holder | claude-5f0493, lease until 2026-11-17T05:14:29Z |
| Last verified commit | a44ae5e |

## Goal

Hosts of a pool whose slice has `dhcp4` (CON-34) obtain and renew IPv4 addresses through DHCP, one client per host with MAC `mac_base + i`. The fps-hosts API gives the agent the leased addresses of the pool, so a later agent change can build the pool at Prepare and filter a DHCP port by those addresses.

## Plan

- [x] 1. DHCPv4 message codec (`dhcp/msg.rs`) and framing (`dhcp/frame.rs`) with unit tests.
- [x] 2. Per host state machine (`dhcp/client.rs`, renew at T1, rebind at T2, expiry, NAK) with unit tests.
- [x] 3. `HostPool::dhcp`, `LeaseTable`, `NeighborTable::add_leased`, kernel port refusal.
- [x] 4. `DhcpHosts` driver over `FrameIo` and `tests/dhcp_port.rs` (in memory server, no root).
- [x] 5. Namespace harness test `tests/dhcp_harness.rs` (root or `sudo -n`, AF_XDP on the client arm) and the contract example test.
- [ ] 6. Wiki page, log entry, checks, PR, CI.

## Done and verified

- Steps 1 to 5: `cargo test -p fps-hosts` passes (113 unit tests, `dhcp_port` 13 tests, `dhcp_harness` 1 test), `cargo clippy -p fps-hosts --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `cargo deny check licenses bans sources` are clean. `cargo check -p fps-runtime -p fps-agent` builds against the changed crate.
- The harness test ran as root on the development host three times (about 3.6 seconds each): 8 hosts lease 10.201.0.100 to .107, renew by unicast at T1, keep the addresses, release, and the router neighbor table maps each address to the host MAC.
- `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Open the PR, wait for CI, set `ready for review`.

## Next action

Push, open the PR against `main`, poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- A DHCP pool is a `HostPool` with a flag and no prefix (`HostPool::dhcp`), so `MacAllocator`, `NeighborTable`, and the agent keep their types. Static address calls return no address for it.
- `LeaseTable` is plain data with a generation number. The `Responder` stays immutable, and the loop rebuilds it from a snapshot when the generation moves.
- The UDP checksum of a received frame is not checked, because a veth peer with checksum offload sends the partial pseudo header checksum.
- No new dependency. `socket2` (dev dependency) gets the feature `all` for `bind_device`.
- The harness test follows the AF_XDP veth test of TRN-11 (self elevation through `sudo -n`, skip with a message, fail on `CI=true`), so it runs in the `rust` job of `ci.yml` with no workflow change.

## Pitfalls

- A kernel sender on a veth pair leaves the UDP checksum partial, so a parser that verifies it drops every reply (found in the harness).
- The sandbox worktree guard refuses commands that mix heredocs, `source`, or `git` with other shell constructs. Write files with the editor tools and run `git` as its own command.

## Host test

Not needed. The namespace test runs in CI (it needs only network namespaces and AF_XDP on veth, which hosted runners have). For a run on a real host, the argv is `cargo test -p fps-hosts --test dhcp_harness -- --nocapture` with `FPS_REQUIRE_DHCP=1`, run as root or with passwordless sudo, timeout 120 seconds, expected result one passed test.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target/hst04 cargo test -p fps-hosts
```

Expected result, every test passes, and the harness test prints `fps-dhcp bound`, `renewed`, `released`, and `server` lines.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-17T04:28Z | claude-5f0493 | claim | 7cacc8b | claimed #574 |
| 2026-11-17T04:59Z | claude-5f0493 | checkpoint | a44ae5e | codec, state machine, lease table, driver, in memory tests |
| 2026-11-17T05:08Z | claude-5f0493 | checkpoint | none | harness test, wiki page, log entry |
