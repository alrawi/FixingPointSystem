## [2026-11-17] ingest | HST-03 ARP and ND for user space backends

HST-03 adds `MacAllocator`, `NeighborTable`, `Responder`, and `NeighborService` to `engine/fps-hosts/`. The allocator keeps the contract rule `mac_base + i` and refuses overlap, multicast, zero, and 48 bit overflow. The responder answers ARP requests and IPv6 neighbor solicitations for the hosts of a port, including ARP probes and duplicate address detection, and hands every other frame back. The service runs it on a `FrameIo` queue. [Hosts](../Hosts.md) describes the package.

Evidence: 20,000 hosts on one port pass the realism host identity check over the in memory frame pair, for IPv4, IPv6, and two pools with an allocated base. A capture with two responders for one address fails the check with the expected conflicts.

What the package found:
- `fps-hosts` now depends on `fps-backend-afxdp` for `FrameIo`, so `Cargo.lock` changes by one line.
- A pool whose first host address equals the router address makes the router's own request an announcement that gets no answer. A pool that shares a link with its router must name the router as `gateway`.
- The AF_XDP backend crate was not touched (bug #588 is fixed there by another worker).

### Shared page notes

None.
