## [2026-11-28] decision | AGT-23: a virtual source pool mapped on the live lease boards

- The vetting agent accepted under D13 the AGT-23 design that its worker proposed. The leases exist only after the datapath starts, so `assemble` gives a client pool that leases a virtual static pool in 240.0.0.0/4, which only encodes the host index the selector picked. `DatapathTransport::connect` and `connect_datagram` map it, by the pool and never by the address range, to the address the host holds on the live lease board at each connect.
- 240.0.0.0/4 (class E), not 100.64.0.0/10: a CGN lab leases from 100.64.0.0/10, so a missed mapping there would connect from another host's real lease silently, while class E is never leased and fails closed.
- A host without a lease fails its session naming dhcp4, the pool and the host index, with no frame sent, and the virtual address reaches no frame, capture, report, label or error text. The DHCP seed is the run's effective seed.
- AGT-23 waits for bug #724 (`LeaseMonitor::boards` in fps-datapath). The deliverable "Pools::addresses reads the lease table" is replaced, and the neighbor table deliverable is obsolete since TRN-37.
- Known debt: the virtual pool encodes a host index as an address because `SourceHosts` and the transport trait live in `engine/fps-runtime/`.

### Shared page notes

None.
