# Handoff TRN-29 UDP over FrameIo

| Field | Value |
|---|---|
| Issue | #630 |
| Branch | `wp/TRN-29` |
| Pull request | opened at the last checkpoint, see the issue |
| Status | in progress |
| Progress | 90% (4 of 5 plan steps, CI pending) |
| Current holder | claude-a7c3d1, lease renewed at each checkpoint |
| Last verified commit | see the session log |

## Goal

A user space UDP layer behind `fps_io::DatagramIo` over Ethernet and IPv4 frames (new crate `engine/fps-udp-own`), wired into `engine/fps-datapath` in place of the UDP `Unsupported` hook, so the UDP modules run on the AF_XDP datapath. Acceptance: the dns module completes its scenarios over the user space UDP layer on a veth pair in CI.

## Plan

- [x] 1. fps-udp-own: wire format (checksums, IPv4, fragmentation on send, fragments rejected on receive), sessions, listener, DatagramIo, tests
- [x] 2. fps-datapath: `FrameHook::poll_tx`, UDP arms per lane, `connect_udp`, `listen_udp`, stats, memory tests
- [x] 3. In memory dns test (no root)
- [x] 4. dns over an AF_XDP veth pair, and with a kernel server on the other end
- [ ] 5. Checks, log entry, PR, CI green, `fps_swarm.py review`

## Done and verified

- `cargo test -p fps-udp-own -p fps-datapath` (all green, as root in the sandbox, so the two veth tests ran), `cargo clippy -p fps-udp-own -p fps-datapath --all-targets -- -D warnings`, `cargo fmt --check`, `cargo deny check`, `cargo test -p fps-backend-afxdp --test datapath_veth` (the TRN-27 test still passes), `plan.py check`, `render --check`, `lint`.
- dns scenarios, 20 rounds: 140 UDP flows and 20 TCP flows, 0 failures over two AF_XDP queues, and against a kernel server on the other end (`engine/fps-datapath/results/sandbox-2vcpu-trn29/`).

## Remaining

CI of the pull request (the `rust` job runs the three dns tests with `CI=true`). The `trn.yml` step and path filters are in the log entry under Shared page notes (coordinator file).

## Next action

Poll the checks of the pull request, fix failures inside `engine/fps-udp-own/` and `engine/fps-datapath/`, then set the status to ready for review and run `fps_swarm.py review 630 --pr <PR>`.

## Blockers and open questions

none

## Decisions taken in this package

- The layer is a `UdpArm` per queue with no thread; the datapath loop calls `receive` and `drain_tx`. A new default method `FrameHook::poll_tx` is the seam for frames that no received frame caused. Reason: the hook trait only reacted to frames, and a sender task must get its datagrams onto the queue on the thread that owns it.
- Sessions are connected (`DatagramIo` is a connected socket), so there is no `bind`, `send_to`, or `recv_from`; a listener accepts a session per peer, as `KernelDatagramListener` does.
- Fragmentation on send is implemented; reassembly is not (received fragments are dropped and counted). No ICMP either way. Both are documented in the crate docs, `Transport.md`, and the log entry.
- The UDP layer depends on `fps-tcp-own` for `Station`, `Pools`, and the Ethernet framing, so MAC addresses and VLAN handling match the stack and the ARP responder.
- `Connector::connect_udp` is synchronous (UDP has no handshake). The old stub was `async` and returned `()`, and nothing used it.
- The dns acceptance test lives in `engine/fps-datapath/tests/` and not in `fps-backend-afxdp`, because adding dev-dependencies to that crate is outside the paths. It runs in the `rust` job of `ci.yml`; the `trn.yml` step is in the Shared page notes.

## Pitfalls

- A frame with a total length of 0 in a test is `Malformed` before its checksum is read: the order in `wire::parse` is length, header checksum, fragment, protocol, UDP length, UDP checksum.
- Dropping a `UdpSession` locks the table of its arm, so never drop one while holding that lock (the accept path creates the session after the lock is released).
- The kernel server test needs the MAC address of the first server host on `dnsb` and its transmit offload off (partial checksums on veth, TRN-26).

## Verify the current state

```bash
cargo test -p fps-udp-own -p fps-datapath          # the veth tests need root or sudo -n
FPS_REQUIRE_AFXDP=1 cargo test -p fps-datapath --test dns_scenarios -- --nocapture
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-a7c3d1 | claim | - | claimed #630 |
| 2026-11-18 | claude-a7c3d1 | checkpoint | 8dd4387 | fps-udp-own and the datapath wiring |
| 2026-11-18 | claude-a7c3d1 | checkpoint | 4ed2be8 | dns scenarios over the datapath, three tests |
| 2026-11-18 | claude-a7c3d1 | checkpoint | next commit | log entry, Transport.md, pull request |
