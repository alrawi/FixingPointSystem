# Handoff TRN-32 Host pools and Ethernet in the own stack

| Field | Value |
|---|---|
| Issue | #622 |
| Branch | `wp/TRN-32` |
| Pull request | #640 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-7c41e9, lease until 2026-11-18T02:56:32Z |
| Last verified commit | 9a9684c (CI green) |

## Goal

Give the own stack (engine/fps-tcp-own) a connection table keyed by the full 4-tuple (flat per-socket cost), Ethernet and 802.1Q framing with MAC = mac_base + i, and neighbor lookup from fps-hosts (read only), so it serves host pools. Update the fps-tcp adapter and tcp_bench for an address pool. Acceptance: the 1,000,000 socket row of tcp_bench gives at least 50,000 probe sessions per second with a pool of 20,000 addresses.

## Plan

- [x] 1. 4-tuple table and address pool in the stack, with unit tests
- [x] 2. Ethernet and VLAN framing, neighbor lookup, with unit tests
- [x] 3. fps-tcp adapter and tcp_bench pool option
- [x] 4. Benchmark run committed, log entry, checks, PR

## Done and verified

Steps 1 to 3: Tuple-keyed table (host.rs), net.rs (Ethernet, VLAN, Station with fps-hosts NeighborTable and ARP responder), pools.rs, OwnStack::with_pools / connect_hosts, tcp_bench flags --own-pool, --own-server-hosts, --own-vlan. `cargo test -p fps-tcp-own -p fps-tcp` passes.

## Remaining

CI on the PR. The 50,000 probe sessions per second acceptance reads 46,500 to 48,600 in this sandbox session, equal to the unchanged base (47,200 to 49,100); rerun on a quiet host (see results host.txt).

(older text) Step 4: run tcp_bench scale at 1,000,000 sockets with --own-pool 20000 and commit results under engine/fps-tcp/results/sandbox-2vcpu-trn32/ with host.txt; clippy, fmt, deny, plan checks; log entry; PR.

## Next action

Wait for CI, then set ready for review. (older) Build the release example and run the scale workload with the pool.
