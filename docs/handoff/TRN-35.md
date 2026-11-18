# Handoff TRN-35 The own stacks resolve their next hop

| Field | Value |
|---|---|
| Issue | #660 |
| Branch | `wp/TRN-35` |
| Pull request | #670 |
| Status | in progress |
| Progress | 90% (4 of 5 plan steps, PR open) |
| Current holder | claude-648711, lease until 2026-11-18T16:34:00Z (renewed at each heartbeat) |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The user space stacks answer ARP and ND for their pools but send no ARP requests or neighbor solicitations, so a peer behind a router works only when the next hop MAC is configured by hand (TRN-26 finding). Resolve the next hop for IPv4 and IPv6 in the datapath, hold frames while it resolves (bounded), retry, age the entries, and count resolutions and failures.

Acceptance: on a veth pair in CI the own stack reaches a kernel peer behind a namespace router with no neighbor entry configured, over IPv4 and IPv6.

## Plan

- [x] 1. Claim, read the code (split.rs, net.rs, host.rs, fps-hosts wire builders)
- [x] 2. `Station` defers the peer MAC (`OwnConfig::resolve_next_hop`), the datapath neighbor cache with ARP and ND requests, hold, retry, aging, counters, unit tests
- [x] 3. Splitter wiring and an in memory test of the datapath (no root)
- [x] 4. Router test on veth pairs and namespaces over IPv4 and IPv6, both directions
- [ ] 5. Checks, log entry, component page, PR, CI

## Done and verified

- `cargo test -p fps-datapath` passes in the sandbox as root: 15 unit tests (11 of the neighbor cache), `next_hop.rs` (9 tests in memory, a router thread between the arms, IPv4, IPv6, VLAN, unanswered gateway), and the existing suites.
- `cargo test -p fps-datapath --test router` runs on veth pairs in three namespaces: the own stack as client and as server through a kernel router, IPv4, 256 KiB echo, requests=1 resolved=1 failed=0. The sandbox kernel has no IPv6 (RTNETLINK operation not supported), so the two IPv6 scenarios skip here and run in CI.
- `plan.py check`, `render --check`, `lint` are clean. `cargo check --tests` of fps-tcp, fps-backend-afxdp, fps-agent passes. The full tests of fps-tcp-own and fps-udp-own pass.
- Log entry `docs/wiki/log/2026-11-18T1600-ingest-trn-35-own-stacks-resolve-their-next-hop.md` and the section "Next hop resolution in the datapath (TRN-35)" of `Transport.md` are written.
- clippy `-D warnings` and fmt clean for fps-datapath, fps-tcp-own, fps-udp-own.

## Remaining

- CI on the PR (the IPv6 router scenarios run only there).

## Next action

Poll the check runs of the PR. When they are green, set the handoff to ready for review and run `fps_swarm.py review 660 --pr <PR>`.

## Blockers and open questions

none

## Decisions taken in this package

- Where the gateway comes from: `HostPool::gateway()` of the pool of the arm (the plan gives it per host pool, `fps-agent` passes it through `HostPoolSlice`). A pool with no gateway resolves an off link destination itself (a flat link or proxy ARP). The lease router of HST-04 (`LeaseTable::router_of`) is not read, because DHCP is not wired into the datapath yet; that is the gap.

## Pitfalls

- Stopping a datapath right after `write_all` and `shutdown` cuts the data in the send buffer short. The router helper of the own server waits for the peer to close first (the first CI run failed once for this).
- The sandbox kernel has no IPv6, so the IPv6 router scenarios run only in CI (they passed there in the first run).

## Verify the current state

```bash
git fetch origin && git log --oneline origin/wp/TRN-35 | head
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-648711 | claim | none | claimed #660 |
