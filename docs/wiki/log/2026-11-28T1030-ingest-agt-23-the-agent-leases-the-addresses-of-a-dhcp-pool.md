## [2026-11-28] ingest | AGT-23 The agent leases the addresses of a DHCP pool

AGT-23 (issue #635, bug #708) lets the agent run a client host pool with `dhcp4` on the AF_XDP backend. `backend::datapath_config` sets `DatapathConfig::dhcp` with the seed of the run, `launch` waits for `Datapath::wait_leases` before any worker starts (a timeout fails the start with the `LeaseTimeout` text of the datapath, which names dhcp4, the pool, and the bound count), `layout` accepts a leasing client pool and refuses a leasing server pool, and the kernel backend refuses the plan at `prepare` with an error that names dhcp4 and, for an unforced agent that fell back from AF_XDP, the cause of the fallback (`backend::check_kernel_dhcp`). [Agent](../Agent.md#dhcp-client-pool) has the design.

Acceptance. `engine/fps-agent/tests/afxdp_dhcp.rs` runs `contracts/examples/tests/dhcp_clients.yaml` (phases shortened to 4 s, objective lowered to 20 sessions per second) through a forced AF_XDP agent in two namespaces with a DHCP server in the router namespace. All 200 clients lease, the last first ACK precedes the first SYN at the server arm, 84 of 84 sessions pass on both sides, every client address that a packet socket on the server arm saw is a leased one, and none lies in 240.0.0.0/4. The `EXPECTED` row of `tests/fallback.rs` for dhcp_clients.yaml now asserts AF_XDP with no fallback on the unforced selection and the dhcp4 refusal on the kernel selection, and the `Refused` outcome is gone from the table.

Decisions.

- The leases exist only after the datapath starts, so a leasing client pool gets a virtual static pool in 240.0.0.0/4 in `assemble` (vetting agent, 2026-11-28T0730). `DatapathTransport` holds a `LeaseMap` exactly when the client pool leases, and maps the encoded host index to `LeaseBoard::addr_of(index)` (the absolute index) of the live boards (`LeaseMonitor::boards`, bug #724) at each connect. A host with no lease fails with dhcp4, the pool, and the host index and sends no frame. The deliverable "Pools::addresses reads the lease table" and the neighbor table rebuild are replaced by this and by the datapath, which follows the generation.
- A leasing pool turns `DatapathConfig::neighbors` on, because the router of a lease is a next hop that the datapath resolves.
- The capture filter reads the live boards through `portfilter::LeaseCell`, which the session fills after `start_backend`. It keeps none before.
- `socket2` (with `all`) is a dev dependency of `fps-agent` for the DHCP server and the packet socket of the test, so `Cargo.lock` gains one edge and no package.

Known debt: the virtual pool encodes a host index as an address because `SourceHosts` and the transport trait live in `engine/fps-runtime/`.

Findings.

- The unit test that the mapping reads the board at each connect uses a fake `LeaseLookup`, because a `LeaseTable` can be filled only inside `fps-hosts` (`set` is crate private). The real boards are exercised by the harness test.
- `family_of` had to treat a pool with `dhcp4` and no prefix as IPv4, or `layout` refused it as having no prefix.
- The disk of the worker machine filled while the test binaries of `fps-agent` linked (about 700 MB each); building one test target at a time and deleting the binaries fixed it.

### Shared page notes

- Testing: add `cargo test -p fps-agent --test afxdp_dhcp` (root or `sudo -n`, `ip`, AF_XDP; honors `CI=true` and `FPS_REQUIRE_AFXDP=1`) to the agent test list: dhcp_clients.yaml through a forced AF_XDP agent with a DHCP server in the router namespace. It runs in the `rust` job of `ci.yml` with no workflow change.
- Transport: no change. The DHCP section already says what the datapath does.
- TRN-31 and Implementation Plan: AGT-23 is complete, so the DHCP dependency of TRN-31 holds and bug #708 closes. The coordinator makes the `fps-agent` test changes of TRN-31 (`TRANSPORT_AVAILABLE` pins) in its pull request; `EXPECTED` in `tests/fallback.rs` no longer has a row that every backend refuses.
- Module Development: no change.
- Cargo.lock: the only shared file that changed, by the dev dependency edge of `fps-agent` on `socket2`.
