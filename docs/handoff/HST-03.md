# Handoff HST-03 ARP and ND for user space backends

| Field | Value |
|---|---|
| Issue | #564 |
| Branch | `wp/HST-03` |
| Pull request | not yet opened |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-9c41d7, lease until 2026-11-17T01:15:10Z |
| Last verified commit | none |

## Goal

Answer ARP and IPv6 neighbor discovery for host pools of user space backends, and assign MAC addresses per host with an allocator that avoids conflicts. DHCP is HST-04. Acceptance: the host identity check of `realism/hosts.py` passes for 20,000 hosts on one port.

## Plan

- [ ] 1. MAC allocator with conflict checks (`mac.rs`)
- [ ] 2. Neighbor table with reverse address lookup, ARP and ND responders on bytes (`neighbor.rs`)
- [ ] 3. Port loop over `fps_backend_afxdp::FrameIo`, tested on the in memory pair
- [ ] 4. 20,000 host capture scored by `realism.hosts`, wiki page, log entry, PR

## Done and verified

Nothing yet.

## Remaining

All steps.

## Next action

Write `engine/fps-hosts/src/mac.rs`.

## Blockers and open questions

none

## Decisions taken in this package

- The vetting agent notes say fat-nuc is not needed. Tests use `fps_backend_afxdp::memory::frame_pair`. We do not edit `engine/fps-backend-afxdp/` (bug #588 is fixed there by another worker).

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-hosts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16 | claude-9c41d7 | claim | none | claimed #564 with 4 plan steps |
