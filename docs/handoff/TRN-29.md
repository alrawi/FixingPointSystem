# Handoff TRN-29 UDP over FrameIo

| Field | Value |
|---|---|
| Issue | #630 |
| Branch | `wp/TRN-29` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-a7c3d1, lease until 2026-11-18T11:06:55Z |
| Last verified commit | none |

## Goal

A user space UDP layer behind `fps_io::DatagramIo` over Ethernet and IPv4 frames (new crate `engine/fps-udp-own`), wired into `engine/fps-datapath` in place of the UDP `Unsupported` hook, so the UDP modules run on the AF_XDP datapath. Acceptance: the dns module completes its scenarios over the user space UDP layer on a veth pair in CI.

## Plan

- [ ] 1. fps-udp-own: wire format (checksums, IPv4, fragmentation on send, fragments rejected on receive), sockets, listener, DatagramIo, unit tests
- [ ] 2. fps-datapath: FrameHook transmit seam, UDP arms per lane, connect_udp and listen_udp, stats, memory tests
- [ ] 3. In memory dns test (no root) in engine/fps-datapath/tests
- [ ] 4. dns over AF_XDP veth test (root through sudo -n)
- [ ] 5. Checks, log entry, PR, CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write engine/fps-udp-own.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cargo test -p fps-udp-own -p fps-datapath
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-a7c3d1 | claim | - | claimed #630 |
