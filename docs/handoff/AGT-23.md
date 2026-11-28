# Handoff AGT-23 The agent leases the addresses of a DHCP pool

| Field | Value |
|---|---|
| Issue | #635 |
| Branch | `wp/AGT-23` |
| Pull request | none yet |
| Status | blocked |
| Progress | 10% (0 of 5 plan steps; deliverable 2 done) |
| Current holder | claude-a23c7e, lease until 2026-11-28T08:28:34Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent leases the addresses of a DHCP host pool through the lease phase of TRN-37, on the user space backend, filters a DHCP port by the leased addresses, and refuses dhcp4 on the kernel backend with the cause of an unforced fallback. dhcp_clients.yaml runs through a forced AF_XDP agent in the namespace harness (bug #708).

## Plan

- [ ] 1. assemble builds HostPool::dhcp; check_af_xdp accepts a DHCP client pool; kernel refusal names dhcp4.
- [ ] 2. AF_XDP backend waits for the lease phase (timeout error names dhcp4, pool, bound count).
- [ ] 3. Pools::addresses reads the lease table; capture filter keeps a session only when an address is leased; neighbor table follows the lease generation.
- [ ] 4. tests/fallback.rs EXPECTED row and the dhcp test; fallback cause in the kernel refusal error.
- [ ] 5. Namespace harness test, docs (Agent page), log entry, PR, CI.

## Done and verified

- Deliverable 2 (kernel refusal carries the fallback cause): `backend::check_kernel_dhcp`, called from `run::prepare_selected` for the kernel backend. New test `the_kernel_refusal_of_a_dhcp_plan_carries_the_cause_of_the_fallback`; the dhcp_clients.yaml row text of EXPECTED follows. `cargo test -p fps-agent --test fallback`: 11 passed. `cargo clippy -p fps-agent --all-targets -D warnings` clean.

## Remaining

Steps 1 to 5 of the plan, which are blocked on one change outside `engine/fps-agent/`:

`fps_datapath::Datapath` keeps its live lease boards private (`Running.leases`), and `Pools::leases()` builds fresh boards, so the agent cannot read the address a leased host holds. The datapath connector needs a leased address as `source`, and the runtime workers pick the source from the pool (`fps-runtime` `source_address`, `pool.ipv4_addr(index)`, which is `None` for a DHCP pool). Needed in `engine/fps-datapath/src/lib.rs`: `LeaseMonitor::boards() -> Vec<ArmBoard>` with `pub struct ArmBoard { pub lane: usize, pub side: Side, pub board: Arc<LeaseBoard> }`, built from `Running.leases` as `LeaseMonitor::report` does.

Planned agent design once it exists: a client DHCP pool gets a virtual static pool (100.64.0.0/10) in `SourceHosts` so the runtime picks an index; `DatapathTransport::connect` and `connect_datagram` map the virtual source to `board.addr_of(index)`; `AfXdpBackend` waits for `wait_leases()` in `launch` with `DhcpSettings` seeded from the plan; `layout` accepts a DHCP client pool and refuses a DHCP server pool; `PortNets` carries a cell that the drive thread fills with the boards after `start_backend`, and the filter keeps a session when `board.index_of(addr)` is `Some`.

## Next action

Apply the datapath accessor (coordinator or a TRN bug), then start step 1.

## Blockers and open questions

The accessor above, outside the paths of AGT-23.

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cd engine/fps-agent && cargo test -p fps-agent --lib
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-28T08:28Z | claude-a23c7e | claim | none | claimed #635 with 5 plan steps |
| 2026-11-28T08:50Z | claude-a23c7e | block | see git log | deliverable 2 done; the rest needs LeaseMonitor::boards in fps-datapath |
