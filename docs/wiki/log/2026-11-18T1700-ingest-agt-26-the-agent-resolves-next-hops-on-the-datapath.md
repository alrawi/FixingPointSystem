## [2026-11-18] ingest | AGT-26 the agent resolves next hops on the datapath

AGT-26 turns the neighbor resolution of the datapath (TRN-35) on in the agent. It edits only `engine/fps-agent/`, its handoff, this entry, and `Agent.md`. It changes no other crate, no workflow, and not `Cargo.lock`.

- `backend::datapath_config` (new) builds the `DatapathConfig` of a lane. When the host pool of either arm carries a gateway (`HostPool::gateway()` of the parsed pool), `neighbors` is `Some(NeighborConfig::default())` and `OwnConfig::resolve_next_hop` is true. Otherwise both stay off and the stack reads its peers from the pools as before. The datapath sets `resolve_next_hop` itself when `neighbors` is set, so the agent only states it for the same sides. The switch is one for both arms; the datapath picks the gateway of each arm from the pool of that arm and asks for the destination itself on an arm whose pool has no gateway.
- DHCP pools: the gap is recorded, not built. `fps-datapath` classifies DHCP frames (`Class::Dhcp`) and answers `Unsupported` for them, and nothing plugs a DHCP hook in, so the agent keeps refusing a DHCP pool on AF_XDP (`layout`). `LeaseTable::router_of` is therefore not read. Once the hook is wired, the next hop of a leased host comes from the lease and the agent has to build the `Route` of that arm from it (a follow up of HST-04 on the datapath).

Evidence. Unit tests on fake hosts (`backend::tests`, no root): the switch for no gateway, one on either pool, and both; and a datapath on in memory links whose client pool has a gateway, where a connect to an address that no pool serves makes the client arm send an ARP request (`neighbor_requests >= 1`, none on the server arm). The acceptance test is `tests/afxdp_router.rs` (root): the agent runs the http1 and dns entries of the example plan for 3 seconds with the client pool on 10.0.0.0/24 (gateway 10.0.0.1) and the server pool on 10.0.1.0/24 (gateway 10.0.1.1), the arms on veth ends `oa` and `ob` in one namespace and a kernel router with forwarding and an empty neighbor table on the other ends. Sandbox run (root, kernel 6.18, debug build, 3.8 s): http1 63 and dns 24 sessions, all passed on both sides, none failed; each arm sent 1 request, got its answer, resolved once, held and released 1 frame, dropped none, failed none, and held only the gateway as an entry; the router held the hosts of both pools afterwards.

Findings:

- The peer of a plan run is always an own stack host: a worker connects to a host of the server pool, and the module on the far side is the one of the plan. A kernel socket as the peer therefore cannot be a plan peer, and the acceptance is shown with the own stack on both arms and the kernel as the router. The kernel socket peer is the router test of TRN-35 (`fps-datapath/tests/router.rs`), which drives the same cache.
- Without the change, the same plan most likely passes too (not run): the pools hold the MAC address of every host, so the frames go straight to the other arm's host and never touch the router. The test therefore asserts the neighbor counters (a request, an answer, a resolution on each arm) and that the router learned the hosts, which only happens when the frames pass through it. A test of a gateway layout that checks the sessions only proves nothing.
- No transmit checksum offload change was needed here: every TCP endpoint is the own stack, which writes complete checksums, and the router only forwards. (A kernel TCP peer needs the offload off on its veth end, see TRN-26 and TRN-35.)

### Shared page notes

- Workflow (coordinator file `.github/workflows/trn.yml`): the new test runs in the `rust` job of `ci.yml` (`cargo test --workspace`, `CI=true`, root). To run it in the `afxdp` job too, add this step after the `AF_XDP backend of the agent` step (the path filters already hold `engine/fps-agent/**`):

  ```yaml
      - name: agent next hop resolution behind a namespace router
        run: cargo test -p fps-agent --test afxdp_router -- --nocapture
        env:
          FPS_REQUIRE_AFXDP: "1"
          FPS_ROUTER_RESULTS: ${{ runner.temp }}/agent-router
  ```

  It needs `ip` and `sysctl` (both on the hosted runner) and no apt package.
- Testing page: `cargo test -p fps-agent --lib backend::` needs no root. `cargo test -p fps-agent --test afxdp_router` needs root (it re-runs through `sudo -n`), `ip`, `sysctl`, and AF_XDP; `FPS_REQUIRE_AFXDP=1` or `CI=true` turns every skip into a failure, and `FPS_ROUTER_RESULTS` receives `agent-router.txt`.
- Transport page, section "Backend selection": after the sentence about the `os_profile` of each host pool, add: "When a host pool of the plan carries a gateway, the agent turns on the neighbor resolution of the datapath, so the peers behind that gateway need no neighbor entry (AGT-26)."
- `Cargo.lock`: unchanged. Module development: nothing changes for module authors. Implementation Plan and sidebar: nothing.
