## [2026-11-18] ingest | TRN-35 the own stacks resolve their next hop

TRN-35 gives the user space datapath a neighbor cache: it sends ARP requests (IPv4) and neighbor solicitations (IPv6) for the next hop of the frames of the stack, holds the frames while it waits, repeats the request, ages the entries, and counts what it does. A peer behind a router is reached with no neighbor entry configured. It edits `engine/fps-datapath/`, `engine/fps-tcp-own/` (a marker MAC address and one setting; `engine/fps-udp-own/` needed no change), its handoff, this entry, and `Transport.md`.

- `fps-tcp-own`: `NEXT_HOP_MAC` (a locally administered address that no pool hands out), `Station::defer_peers()`, and `OwnConfig::resolve_next_hop`. A station with deferred peers answers the marker for every peer, in the table or not, so a connect or an incoming SYN no longer needs a neighbor entry, and the frames leave with the marker as their Ethernet destination. Off by default, so every existing user of the stack is unchanged.
- `fps-datapath`: `neighbor.rs` with `NeighborConfig`, `Route`, and the cache; `Splitter::with_next_hop`; `DatapathConfig::neighbors` (`None` by default). The splitter replaces the marker on every frame that leaves, TCP and UDP alike. A frame to a known next hop goes at once, an unknown one is held and a request goes out from the host that sends (with the VLAN tag of the port), the request is repeated every second, 3 requests in all, and then the held frames are dropped and the next hop is left alone for 3 seconds. The answers that the cache reads are ARP replies and requests from the next hop and neighbor advertisements with hop limit 255 and a good checksum; entries are created only for frames of the stack, and an entry lives 60 seconds and is then asked for again. The hold is bounded per next hop (32), in all (1,024), and the table (65,536). `Connector::connect` takes a destination that no pool serves when the datapath resolves, because the peer is behind a router.
- Where the gateway comes from: `HostPool::gateway()` of the pool of each arm (`Route::of`), which the plan gives for each host pool. A pool without a gateway asks for the destination itself. Nothing is invented: a DHCP pool has no prefix and no gateway, and the lease router of HST-04 is not read because the datapath runs no DHCP yet (see the gaps).
- Counters in `FrameCounters`: `neighbor_requests`, `neighbor_answers`, `neighbor_resolved`, `neighbor_failed`, `neighbor_refreshed`, `neighbor_expired`, `neighbor_held`, `neighbor_released`, `neighbor_dropped`, `neighbor_table_full`, and the gauge `neighbor_entries`.

Tests: 11 unit tests in `neighbor.rs`, 9 in `tests/next_hop.rs` (no root: the splitter against a hand played neighbor, and a whole datapath with a router thread between the arms, TCP echo over IPv4, IPv6, and a VLAN, and a gateway that never answers), 1 in `net_tests.rs`, and the acceptance test `tests/router.rs` with four scenarios on veth pairs in three namespaces (own stack as client and as server, IPv4 and IPv6, 256 KiB echo checked byte by byte, kernel router with forwarding, kernel socket peer, no neighbor entry anywhere). The IPv4 scenarios ran in the sandbox with one request, one resolution, no failure, and one held frame released. The sandbox kernel has no IPv6, so the two IPv6 scenarios could not run there; they run in CI, and `CI=true` turns a skip into a failure.

Findings:

- A kernel router learns the own host from the ARP request or the neighbor solicitation of the own stack (the kernel creates a neighbor entry for a sender that asks for one of its addresses), so with the own stack as client the splitter answers nothing from the router. With the own stack as server the router asks and the splitter answers (`arp_replied` and `nd_replied`).
- The splitter must not hold a frame that follows frames that the queue refused: the stack offers the refused ones again, and a frame held at the first offer would be held twice. `send_batch` sends the frames in runs and lets the cache decide about a frame only after the frames in front of it are on the queue (a test pins this with a queue of depth 2).
- A held frame counts as taken: `send_batch` returns it in the count, and the stack drops it from its queue. Its fate (released, dropped when the hold overflows, dropped when the resolution fails) is in `neighbor_released` and `neighbor_dropped`. TCP sends it again when it is lost, as it does for any other loss.
- In this mode the UDP layer sends its replies to the next hop and not to the source MAC of the frame that it answers, as it did when the neighbor came from a table. A peer behind a router gets its answer through the router, which is what an IP host does.
- The cost per frame in this mode is one parse of the addresses and one hash lookup of the next hop. It is off by default and has not been measured on the veth benchmark of TRN-27.

Gaps, not worked around:

1. `fps-agent` (`backend.rs`) starts the datapath with `DatapathConfig::default()`, so an agent run keeps reading peers from the pools. AGT-25 or a follow up sets `neighbors: Some(NeighborConfig::default())` when the plan's host pools carry a gateway.
2. DHCP pools: the lease router (`LeaseTable::router_of`) is the gateway of a leased host. It becomes available when the DHCP hook is wired into the datapath; then `Route` can be built from the lease and not from the pool.
3. No reachability probing and no ICMP redirects: an entry is trusted for its lifetime. A gateway that changes its MAC address inside it is found when it sends a request of its own or when the entry ages out.

### Shared page notes

- Workflow (coordinator file `.github/workflows/trn.yml`): the router test runs in the `rust` job of `ci.yml` (`cargo test --workspace`, `CI=true`, root), so the acceptance is covered without a change. To run it in the `afxdp` job too and to start that job when only the datapath changes, add this step after the datapath steps, and add `engine/fps-datapath/**`, `engine/fps-tcp-own/**`, and `engine/fps-udp-own/**` to both path filters if they are not there yet:

  ```yaml
      - name: next hop resolution behind a namespace router
        run: cargo test -p fps-datapath --test router -- --nocapture
        env:
          FPS_REQUIRE_AFXDP: "1"
          FPS_ROUTER_RESULTS: ${{ runner.temp }}/router
  ```

  The test needs `ip` and `sysctl` (both are on the hosted Ubuntu runner) and IPv6 in new namespaces (the runner has it). No apt package is installed.
- Testing page: `cargo test -p fps-datapath --test next_hop` needs no root. `cargo test -p fps-datapath --test router` needs root (it re-runs through `sudo -n`), `ip`, `sysctl`, and AF_XDP; `FPS_REQUIRE_AFXDP=1` or `CI=true` turns every skip into a failure, `FPS_ROUTER_KIB` (256) sets the echo size, and `FPS_ROUTER_RESULTS` receives `router-<scenario>.txt`. A kernel without IPv6 skips the two IPv6 scenarios outside CI.
- Module development (`Module-Development.md`, "Watch out for"): not affected. A module sees no new field; a host pool gateway already exists in the plan.
- `Cargo.lock`: unchanged. No new dependency, and no root `Cargo.toml` edit.
- Implementation Plan / sidebar: nothing.
- Transport page: this package added "Next hop resolution in the datapath (TRN-35)" and changed two statements of the TRN-30 section that said the stack sends no neighbor solicitation.
