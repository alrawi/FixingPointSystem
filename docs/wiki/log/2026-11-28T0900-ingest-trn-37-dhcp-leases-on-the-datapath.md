## [2026-11-28] ingest | TRN-37 DHCP leases on the datapath

TRN-37 (issue #720, bug #708) runs the DHCP clients of HST-04 on the datapath. `DatapathConfig::dhcp` turns them on for the arms whose pool leases (`HostPool::dhcp`), each arm gets `DhcpHosts` as its DHCP hook, and `Datapath::wait_leases` (or `leases().wait_async()`) is the lease phase: it ends when every lease is bound or at the timeout with `LeaseTimeout`, an error that names dhcp4 and the bound count of each pool. A new `LeaseBoard` in `fps-tcp-own` carries the lease table with its generation. The stations of the own TCP stack and of the UDP layer, the ARP responder of the splitter, the connector, and the route of the arm read the board and follow the generation, so the route takes its subnet and router from the lease. [Transport](../Transport.md) has the design under "DHCP leases on the datapath (TRN-37)".

Acceptance. In the QA-01 harness with a DHCP server in the router namespace, `engine/fps-datapath/tests/dhcp_harness.rs` binds eight client leases and runs eight own stack sessions of 64 KiB from the leased addresses to the static server pool through the kernel router, none failed. `tests/dhcp_memory.rs` covers the same on in memory links with no root, and the timeout error ("dhcp4: 0 of 3 leases bound after 400 ms").

Decisions.

- `HostSlice`, `Pools`, and `Lane` keep their fields, because `fps-agent` builds them with struct literals and AGT-23 owns that crate. The lease table travels as `Leases` (`Pools::leases()`) beside them, `OwnStack::with_links_pools_and_leases` takes it, and `HostSlice::address_leased` reads a `LeaseTable`. Adding a field to any of them would break the build of `fps-agent`.
- The board publishes one table clone per pass of the queue and builds its `Responder` snapshot lazily once per generation, so the hook does not rebuild a table per lease.
- The route of an arm follows the first lease with a mask and the first with a router. Per host routes are not built.
- `PoolError::Board`, `DatapathError::{DhcpOff, DhcpSetup, LeaseTimeout}` are new. `Pools::tables()` now accepts a DHCP pool (empty leased table) where it returned an error before, which is why `DhcpOff` exists: a leasing pool with no settings would otherwise run with no hosts.

Findings.

- The harness has the two arms in two namespaces, and one process cannot open AF_XDP on both. The test moves the server arm interface (`eth0` of the server namespace) into the client namespace as `eth1`, which leaves the router side as the harness made it. No change to `tests/harness/` was needed.
- `fps-hosts` needed no change.

### Shared page notes

- Testing: add `cargo test -p fps-datapath --test dhcp_memory` (no root) and `--test dhcp_harness` (root or `sudo -n`, `ip`, AF_XDP; honors `CI=true` and `FPS_REQUIRE_AFXDP=1`) to the transport tests line, with one sentence each as in Transport "DHCP leases on the datapath (TRN-37)". Both run in the `rust` job of `ci.yml` with no workflow change.
- Agent (AGT-23): use `DatapathConfig::dhcp`, `Datapath::wait_leases` or `leases().wait_async()`, and read `Datapath::leases().report()` for the error text. The route and the neighbor table follow the leases inside the datapath, so the agent builds no `Route` and no `NeighborTable` for a DHCP arm. `Pools::leases()` gives the boards if the agent needs the addresses (`LeaseBoard::addr_of`, `snapshot().leases()`) for the capture filter.
- Module Development: no change.
- Work Packages and Implementation Plan: no change.
