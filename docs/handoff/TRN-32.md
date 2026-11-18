# Handoff TRN-32 Host pools and Ethernet in the own stack

| Field | Value |
|---|---|
| Issue | #622 |
| Branch | `wp/TRN-32` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-7c41e9, lease until 2026-11-18T02:56:32Z |
| Last verified commit | none |

## Goal

Give the own stack (engine/fps-tcp-own) a connection table keyed by the full 4-tuple (flat per-socket cost), Ethernet and 802.1Q framing with MAC = mac_base + i, and neighbor lookup from fps-hosts (read only), so it serves host pools. Update the fps-tcp adapter and tcp_bench for an address pool. Acceptance: the 1,000,000 socket row of tcp_bench gives at least 50,000 probe sessions per second with a pool of 20,000 addresses.

## Plan

- [ ] 1. 4-tuple table and address pool in the stack, with unit tests
- [ ] 2. Ethernet and VLAN framing, neighbor lookup, with unit tests
- [ ] 3. fps-tcp adapter and tcp_bench pool option
- [ ] 4. Benchmark run committed, log entry, checks, PR

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read engine/fps-tcp-own/src/host.rs and lib.rs, then change the table key.
