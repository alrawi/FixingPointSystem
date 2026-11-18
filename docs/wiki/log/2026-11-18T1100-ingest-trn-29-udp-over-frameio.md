## [2026-11-18] ingest | TRN-29 UDP over FrameIo

TRN-29 adds `engine/fps-udp-own`, a user space UDP layer behind `fps_io::DatagramIo` over Ethernet and IPv4 frames, and plugs it into `engine/fps-datapath` in place of the UDP `Unsupported` hook. The dns scenarios complete over it on an AF_XDP veth pair. It edits only `engine/fps-udp-own/`, `engine/fps-datapath/`, its handoff, this entry, and `Transport.md`; `Cargo.lock` changed for the new crate and the dev-dependencies of `fps-datapath`.

- `fps_udp_own::UdpArm` is the layer of one queue. It serves the hosts of a `fps_tcp_own::net::Station` (the host pools, their MAC addresses, the neighbors, the VLAN) and has no thread and no kernel socket. The loop calls `receive(frame)` for each IPv4 UDP frame and `drain_tx(max, sink)` for the frames that sessions queued. Tasks call `connect(source, destination)` (ephemeral port per client host, from 32768 to 60999 as Linux, a 4 tuple table) and `UdpListener` (`bind`, or `new` and `attach` for several arms, `accept` gives the session and the peer address). `UdpSession` is `DatagramIo`: connected, as `KernelDatagram` is, with a receive queue of 64 datagrams, truncation with the full length returned, `NotConnected` after `close`, and a bounded send queue that makes `poll_send` wait (`tx_blocked`).
- `wire.rs`: IPv4 header and UDP checksums computed on send and verified on receive (a received UDP checksum of zero means none, a computed zero goes out as `0xFFFF`), Ethernet padding to 60 bytes, IPv4 fragmentation on send for datagrams above the MTU (8 byte multiples, one identification, the UDP checksum over the whole datagram). Received fragments are not reassembled: they are dropped and counted (`rx_fragments`). Documented in the crate docs; no module sends datagrams above 1,472 bytes. No ICMP in either direction, no ARP of its own (the neighbor table gives the MAC address, and a server session answers a peer that the table lacks at the source MAC of its first frame), IPv4 only, and ports 67 and 68 are DHCP and never reach the layer.
- `fps-datapath`: `FrameHook::poll_tx` is a new method with a default that sends nothing, and the splitter calls it on every pass of its queue, so a hook can send frames that no received frame caused (the datagrams of tasks). `Replies::room` and `Replies::push_vec` are new. `Datapath::start` builds one `UdpArm` per arm from `Pools::stations()` (the same MAC addresses as the stack and the ARP responder), lowers its MTU to what the queue carries, and puts it behind the UDP hook. `Connector::connect_udp(source, destination)` (sync now, it returns a `DatapathDatagram`) and `Datapath::listen_udp(port)` (returns a `UdpListener`) replace the stubs; `DatapathConfig::udp` sets the layer, and `None` turns it off (the calls say `Feature::Udp`, and the UDP hook of `DatapathConfig::hooks` takes the frames). `LaneStats` has `udp_client` and `udp_server`. ND and DHCP hooks are unchanged.

Tests: `engine/fps-udp-own` has 8 unit tests of the wire format (the RFC 1071 example, a flipped bit in every byte caught by a checksum, zero checksums, fragments) and 17 tests of the layer between two arms in memory (request and response, MACs and padding, VLAN tags, ephemeral ports per host with wrap and exhaustion, connect errors, close and reopen, truncation, fragments and MTU, corrupted frames by cause, no listener, full queues, a blocked sender, one listener on two arms, a peer outside the neighbor table). `engine/fps-datapath/tests/udp_memory.rs` has 6 tests (two lanes with 40 echo sessions of every size up to one frame, UDP and TCP on the same port, fragments, an MTU lowered by the queue, ports freed by drop, settings). `engine/fps-datapath/tests/dns_scenarios.rs` has the acceptance tests:

- `dns_scenarios_complete_over_the_datapath_in_memory` (no root): every flow of the seven dns scenarios, both sides through the datapath, UDP flows over the UDP layer and the TCP flow of `tcp_fallback` over the own stack, three rounds, with the messages parsed and compared with the table of `modules/dns/tests/scenarios.rs` (question, rcode, answer types, truncation, `server_ip`) and the bytes of both sides compared.
- `dns_scenarios_complete_over_af_xdp_on_a_veth_pair` (root through `sudo -n`, skips without, fails with `CI=true` or `FPS_REQUIRE_AFXDP=1`): the same run with an AF_XDP queue on each end of a veth pair as the two arms of one lane.
- `dns_client_over_af_xdp_completes_against_a_kernel_server` (same rules): the client side of every flow over AF_XDP on `dnsa`, the server side of every flow over kernel sockets (`fps-backend-kernel`) bound to the first server host on `dnsb`. The kernel checks the checksums and the framing that the layers send, resolves the client hosts with ARP (the splitter answers), and its own transmit checksum offload is off through the ethtool ioctl (the finding of TRN-26).

Result (2 vCPU sandbox, debug build, 20 rounds): 140 UDP flows and 20 TCP flows with 0 failures in 0.52 s over two AF_XDP queues (copy mode) and in 0.54 s against the kernel server; 0 bad checksums, 0 malformed frames, 0 drops. Files in `engine/fps-datapath/results/sandbox-2vcpu-trn29/`. The CI numbers come from the pull request.

Findings:

- The kernel server needs the MAC address of the first server host on its interface (`ip link set dnsb address`) and a route to the client prefix; ARP for the client hosts is answered by the splitter, so no static neighbor entry is needed.
- `OwnStack::connect_hosts` works with a lane whose server arm is an unused in memory queue, so a client only run against a kernel peer needs no stack with one side.
- The `Endpoint::receive` loop of `fps-tcp-own` runs `recv_batch` until it returns 0, so the splitter counts the frames that hooks queued in the count that it returns (documented on `FrameIo::recv_batch` of the splitter): without it a lane that only sends datagrams would sleep as if idle.

### Shared page notes

- Workflow (coordinator file `.github/workflows/trn.yml`): the `afxdp` job runs `cargo test -p fps-backend-afxdp`, which does not run the tests of `fps-datapath`. Add a step to the `afxdp` job, and add the paths below to both path filters (`push` and `pull_request`): `engine/fps-udp-own/**` and `modules/dns/**` (the dns test loads the manifest and the scenarios of the module). `engine/fps-datapath/**` and `engine/fps-tcp-own/**` are there already.

  ```yaml
      - name: dns scenarios over the user space datapath
        run: cargo test -p fps-datapath --test dns_scenarios -- --nocapture
        env:
          FPS_REQUIRE_AFXDP: "1"
          FPS_DNS_RESULTS: ${{ runner.temp }}/dns
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: dns-datapath-results
          path: ${{ runner.temp }}/dns
          if-no-files-found: ignore
  ```

  The `rust` job of `ci.yml` (`cargo test --workspace`, `CI=true`) runs the three tests too, because `CI=true` makes them fail instead of skip, so the acceptance is covered without the step; the step keeps the numbers as an artifact and starts the test when only the UDP crate changes. The tests need `ip` and `sudo -n` (the hosted runner has both) and no `ethtool` (the ioctl is in the test).
- Testing page: `cargo test -p fps-udp-own` and `cargo test -p fps-datapath --test udp_memory` need no root. `cargo test -p fps-datapath --test dns_scenarios` runs the in memory test everywhere and the two veth tests as root (`sudo -n` otherwise); `FPS_DNS_REPS` (5) sets the rounds and `FPS_DNS_RESULTS` a directory for `dns-veth.txt` and `dns-veth-kernel.txt`.
- Module development (`Module-Development.md`, "Watch out for"): add this entry. **A UDP flow that sends more than 1,472 bytes in one datagram is not heard on the AF_XDP backend.** The user space UDP layer (TRN-29) fragments large datagrams on send and does not reassemble received fragments, so a peer that fragments is dropped and counted (`rx_fragments`). Keep datagrams of a module at or below 1,472 bytes of payload (an MTU of 1,500), as the dns module does with its 512 byte limit and the TCP fallback. UDP on ports 67 and 68 is DHCP and does not reach the module on this backend either. Package: TRN-29.
- `Cargo.lock` changed (the new crate, and the dev-dependencies `fps-backend-kernel`, `fps-flow`, `fps-interp`, `fps-io`, `fps-module-dns`, and `libc` of `fps-datapath`). No root `Cargo.toml` edit was needed (the glob `engine/*` picks the crate up).
- API gaps, not worked around (these belong to packages that own the paths):
  1. `fps-tcp-own`: `Station` is built from `Pools::stations()` for each arm in the UDP layer, and `fps-datapath::responders` builds the same tables a second time (TRN-27 note 3). One `Pools::tables()` would remove both copies.
  2. `fps-agent` and the modules: the agent opens UDP sessions through `DatagramOpener` with the kernel backend. A datapath opener needs `Connector::connect_udp(source, destination)` and `UdpListener::accept`, which the test shows (`UdpClient` and `UdpServer` in `dns_scenarios.rs`): the interpreter side needs no change. The choice of the source and server host for a `connect` without a host (what the test does with a round robin over the pools) is an agent wiring decision (TRN-12 or AGT-24).
  3. `fps-hosts`: DHCP on UDP 67 and 68 is classified before the UDP hook, so HST-04 `DhcpHosts` plugs into `Hooks::dhcp`; nothing changed there.
  4. ICMP: the layer sends no port unreachable and reads no ICMP, because the classifier drops ICMP. A module that waits for `ConnectionRefused` on UDP (the kernel backend reports it) will time out instead on this backend. Add an ICMP hook to `Hooks` when a module needs it.
- Implementation Plan / sidebar: nothing. TRN-30 (IPv6 in both layers) plugs into `UdpArm::connect` (it returns `Unsupported` for IPv6 today) and the `wire` module.
