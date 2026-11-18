# Handoff TRN-35 The own stacks resolve their next hop

| Field | Value |
|---|---|
| Issue | #660 |
| Branch | `wp/TRN-35` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-648711, lease until 2026-11-18T16:34:00Z (renewed at each heartbeat) |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The user space stacks answer ARP and ND for their pools but send no ARP requests or neighbor solicitations, so a peer behind a router works only when the next hop MAC is configured by hand (TRN-26 finding). Resolve the next hop for IPv4 and IPv6 in the datapath, hold frames while it resolves (bounded), retry, age the entries, and count resolutions and failures.

Acceptance: on a veth pair in CI the own stack reaches a kernel peer behind a namespace router with no neighbor entry configured, over IPv4 and IPv6.

## Plan

- [ ] 1. Claim, read the code (split.rs, net.rs, host.rs, fps-hosts wire builders)
- [ ] 2. `Station` defers the peer MAC (`OwnConfig::resolve_next_hop`), the datapath neighbor cache with ARP and ND requests, hold, retry, aging, counters, unit tests
- [ ] 3. Splitter wiring and an in memory test of the datapath (no root)
- [ ] 4. Router test on veth pairs and namespaces over IPv4 and IPv6, both directions
- [ ] 5. Checks, log entry, component page, PR, CI

## Done and verified

nothing yet

## Remaining

everything above

## Next action

Implement step 2.

## Blockers and open questions

none

## Decisions taken in this package

- Where the gateway comes from: `HostPool::gateway()` of the pool of the arm (the plan gives it per host pool, `fps-agent` passes it through `HostPoolSlice`). A pool with no gateway resolves an off link destination itself (a flat link or proxy ARP). The lease router of HST-04 (`LeaseTable::router_of`) is not read, because DHCP is not wired into the datapath yet; that is the gap.

## Pitfalls

none yet

## Verify the current state

```bash
git fetch origin && git log --oneline origin/wp/TRN-35 | head
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18 | claude-648711 | claim | none | claimed #660 |
