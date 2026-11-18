# Handoff TRN-30 IPv6 in the own stack

| Field | Value |
|---|---|
| Issue | #631 |
| Branch | `wp/TRN-30` |
| Pull request | #665 |
| Status | ready for review |
| Progress | 100% |
| Current holder | claude-1d4805, lease until 2026-11-18T13:31:09Z |
| Last verified commit | bedbadd |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

IPv6 for the own TCP stack (`fps-tcp-own`), the user space UDP layer (`fps-udp-own`), and the datapath (`fps-datapath`, which answers ND for the host pools). The loss workload (TRN-22) and the dns scenarios (TRN-29) pass over IPv6 on a veth pair in CI.

## Plan

- [x] 1. Read the code of the three crates and decide the address and header types.
- [x] 2. IPv6 in fps-tcp-own (wire, host, pools, net).
- [x] 3. IPv6 in fps-udp-own.
- [x] 4. ND in fps-datapath and the IPv6 connect and listen calls.
- [x] 5. Tests in memory and on a veth pair (dns scenarios and loss workload over IPv4 and IPv6).
- [ ] 6. Checks, log entry, Transport page, pull request, CI.

## Done and verified

Step 5: `cargo test -p fps-datapath` passes locally with `FPS_REQUIRE_AFXDP=1` (root in the sandbox): dns scenarios over IPv6 in memory and on a veth pair, the loss workload of `fps-tcp` over IPv4 and IPv6 in memory and on a veth pair (a lossy queue around each AF_XDP arm drops 2 percent of the frames). The kernel server over IPv6 test cannot run here (the sandbox kernel has no IPv6), so only CI shows it. Log entry and Transport page written; `plan.py check`, `render --check`, `lint`, `make lint-trn test-trn` pass.

Step 4: classifier (Ipv6Tcp, Ipv6Udp, Ipv6Other, Nd), splitter answers neighbor solicitations of the host pools through the Responder (counters nd_replied and nd_ignored), Connector connects TCP and UDP by address family (`Feature::Ipv6` and `Feature::Nd` removed). `cargo test -p fps-datapath --test memory --lib` passes (3 new IPv6 tests).

Step 5 so far: dns scenarios over IPv6 in memory and on a veth pair with AF_XDP on both ends pass locally (root, ethtool offload ioctl). The kernel server over IPv6 test needs a kernel with IPv6; this sandbox has none, so it runs only in CI.

Step 3: fps-udp-own wire (IPv6 header, pseudo header, fragment header, required checksum) and arm (IpAddr keys, connect by family); 22 layer tests (5 new IPv6) and 15 wire tests (7 new) pass, clippy clean.

Step 2: wire, net, pools, host, lib of fps-tcp-own speak IPv4 and IPv6; `cargo test -p fps-tcp-own` passes (163 unit, 30 integration, 6 new IPv6 integration tests), clippy clean.

## Remaining

Open the pull request, watch CI (the dependents `fps-tcp`, `fps-agent`, `fps-backend-afxdp` build only there, the sandbox disk is too small), fix failures.

## Next action

Open the pull request and poll CI.

## Blockers and open questions

The sandbox disk is nearly full (about 600 MB free), so builds use few jobs and no debug info.

## Decisions taken in this package

- `Tuple` and `Conn` hold `IpAddr`; a connection grows by 24 bytes and a table entry by 28 (the Conn budget test went from 176 to 200).
- A dual stack pool connects over IPv4 by default; `OwnStack::connect_addrs` picks the family.
- Over IPv6, UDP receive drops fragments and a zero checksum; no extension headers, no ICMPv6 errors.
- The datapath sends no neighbor solicitation; next hop resolution is TRN-35.

## Pitfalls

- The UDP checksum is required over IPv6; zero is invalid.
- The IPv6 traffic class bits are not covered by a checksum, so a bit flip test must skip them.

## Verify the current state

```bash
git log --oneline -3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T12:01Z | claude-1d4805 | claim | none | claimed issue 631 |
