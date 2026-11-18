## [2026-11-18] ingest | TRN-27 the user space datapath on AF_XDP queues

TRN-27 added `engine/fps-datapath`, the loop per queue that runs the own stack on AF_XDP queues. It does not set `TRANSPORT_AVAILABLE`, does not change the backend selection, and does not edit `fps-agent`, `fps-backend-kernel`, `fps-tcp-own`, or `fps-hosts`.

- A `Lane` is one thread: the queue of the client arm, the queue of the server arm, and the host pools they serve. `OwnStack` owns both links of its pair, so this is the shape the stack has. `split_pools` cuts the pools into lanes.
- The `Splitter` wraps a queue's `FrameIo` and is a `FrameIo`, so the stack sees IPv4 TCP only. ARP goes to the `Responder` of `fps-hosts` (reply from the same MAC addresses that the stack uses, sent on the queue that took the request). ND, DHCP, and UDP go to a `FrameHook` that answers `Unsupported` until TRN-29, TRN-30, and the DHCP wiring plug in, and each is counted. Wrong VLAN tags, short frames, and other protocols are counted and dropped.
- The calls mirror `fps-backend-kernel`: `Datapath::connector()` gives a `Clone` `Connector` with `connect(source, destination)`, `Datapath::listen(port)` gives a `Listener` with `accept`, and the session is the `OwnSession` (`SessionIo`). IPv6 and UDP calls return `DatapathError::Unsupported` with the package that adds them.
- One thread per lane polls `OwnStack::pump`. `OwnStack::drive` sleeps until a timer or a notification, and nothing notifies it when a frame arrives from the wire.
- The acceptance test is `engine/fps-backend-afxdp/tests/datapath_veth.rs`: a namespace with a veth pair, an AF_XDP queue on each end as the two arms of one lane, 2,000 sessions held and 10,000 short sessions from 64 tasks, every byte checked. It lives in `fps-backend-afxdp` with `fps-datapath` as a dev-dependency (Cargo allows the cycle), so the `afxdp` job of `trn.yml` runs it with `FPS_REQUIRE_AFXDP=1`, and the `rust` job runs it where `sudo -n` works.

Result on the hosted runner (the `afxdp` job, debug build, PR 650): 10,000 short sessions with 2,000 held took 1.45 s, which is 6,908 sessions per second, with 4,001 sockets held, no failure, and no wrong byte, on two AF_XDP queues in copy mode. The sandbox (2 vCPU) gives 3,100 to 3,200 in a debug build and 16,000 to 17,000 in a release build.

Results are in `engine/fps-datapath/results/`: `ci-ubuntu-24.04-trn27/` from the hosted runner (debug build, as `cargo test` builds), `sandbox-2vcpu-trn27/` (debug) and `sandbox-2vcpu-trn27-release/` (release, 50,000 short sessions) from the 2 vCPU sandbox. The rate is bounded by the polling thread (it sleeps 30 microseconds when idle) and by the veth copy mode path of the kernel; the table in `Transport.md` has the numbers and the reading.

Tests: `engine/fps-datapath/tests/memory.rs` (7 tests, no root), the unit tests of `classify.rs` (4), and the veth test.

### Shared page notes

- Workflow (coordinator file `.github/workflows/trn.yml`): no new job is needed. The `afxdp` job runs `cargo test -p fps-backend-afxdp -- --nocapture` with `FPS_REQUIRE_AFXDP=1`, which includes `datapath_veth`. To keep the numbers as an artifact, change its step to:

  ```yaml
  - name: AF_XDP veth test
    run: cargo test -p fps-backend-afxdp -- --nocapture
    env:
      FPS_REQUIRE_AFXDP: "1"
      FPS_DATAPATH_RESULTS: ${{ runner.temp }}/datapath
  - uses: actions/upload-artifact@v4
    if: always()
    with:
      name: datapath-veth-results
      path: ${{ runner.temp }}/datapath
      if-no-files-found: ignore
  ```

  If the coordinator prefers a job of its own, the line is `cargo test -p fps-backend-afxdp --test datapath_veth -- --nocapture` with `FPS_REQUIRE_AFXDP: "1"`. The trn.yml path filters list `engine/fps-backend-afxdp/**` but not `engine/fps-datapath/**`, so a change that touches only the datapath does not start the `afxdp` job: add `engine/fps-datapath/**` to both filters (`push` and `pull_request`), and `engine/fps-tcp-own/**` too, because the veth test runs that stack.
- Testing page: `cargo test -p fps-datapath` runs the in memory tests and needs no root. `cargo test -p fps-backend-afxdp` also runs `datapath_veth` (root through `sudo -n`; `FPS_DP_SESSIONS`, `FPS_DP_HOLD`, `FPS_DP_CONCURRENCY`, `FPS_DATAPATH_RESULTS`).
- `Cargo.lock` changed: the new crate, and the dev-dependencies of `fps-backend-afxdp` on `fps-datapath`, `fps-tcp-own`, and `tokio`. No root `Cargo.toml` edit was needed (the glob `engine/*` picks the crate up).
- `fps-tcp-own` API that the datapath needs and does not have (exact changes, for TRN-12 or a package that owns `engine/fps-tcp-own/`):
  1. `OwnSession::local_addr(&self) -> Option<SocketAddr>` and `peer_addr`, so `Listener::accept` can return `(session, peer)` as `KernelListener::accept` does. The host knows the tuple of each connection (`host::Tuple`), so the accessor reads it under the lock.
  2. `OwnStack::unlisten(&self, port: u16)`, so a listener that is dropped frees its port. Today `listen` on a port fails for the life of the stack.
  3. `Pools::tables(&self) -> Result<(NeighborTable, NeighborTable), PoolError>` (or `Built` carrying them). `Pools::stations` builds the two `NeighborTable`s and keeps them inside the `Station`s, so `fps-datapath::responders` repeats the ledger claims (client block first, then server) to get the same MAC addresses for its ARP `Responder`. A test pins the two against each other, but one source would remove the duplicate.
  4. A way to take frames for one side from outside, for example `OwnStack::input(side, frame)` and `OwnStack::take_tx(side)`, would let a datapath run one stack per arm without the `FrameIo` wrapper and without a second thread per lane. The wrapper is enough for TRN-27.
- Module development: not affected.
- Watch out for (Module-Development.md "Watch out for"): not applicable, this package changes no module interface.
- Implementation Plan / sidebar: nothing. TRN-12 may edit `engine/fps-datapath/` and needs the points above, and TRN-29 and TRN-30 plug into `FrameHook` and `Lane`.
