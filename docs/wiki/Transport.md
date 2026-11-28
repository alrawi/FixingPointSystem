# Transport

This page describes the transport backends of the TRN workstream, which implement the `SessionIo` and `DatagramIo` seams of [Engine](Engine.md) over real networks. TRN-01 created the first backend, the kernel TCP backend in `engine/fps-backend-kernel`. Later TRN packages add kernel UDP, operating system profiles, TLS, and the AF_XDP and DPDK backends that the [Implementation Plan](Implementation-Plan.md#target-repository-layout) lists. TRN-04 added the TLS wrapper, see [TLS module](#tls-module), and TRN-11 added the AF_XDP frame input and output, see [AF_XDP backend](#af_xdp-backend).

## Crates

The workspace glob in the root `Cargo.toml` picks up every directory under `engine/`, so a new backend crate needs no edit there. The table lists the backend crates that exist today.

| Crate | Contents | Next package |
|---|---|---|
| [`fps-backend-kernel`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-backend-kernel) | `Connector`, `KernelListener`, and `KernelStream`, which implement `SessionIo` over kernel TCP sockets, the operating system profiles of TRN-03, and `KernelDatagram`, `KernelDatagramListener`, and the two datagram openers of TRN-02, see [Kernel UDP backend](#kernel-udp-backend) | TRN-04 added TLS, see below |
| [`fps-tcp`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-tcp) | `TcpStack`, the bench workloads `session_rate`, `throughput`, and `socket_scaling`, `KernelStack` and `KernelSession`, the kernel loopback reference stack that implements `SessionIo`, and the command `tcp_bench`, see [TCP stack bench harness](#tcp-stack-bench-harness) | TRN-18 added the own stack `OwnStack`, see [The own TCP stack](#the-own-tcp-stack), which TRN-21 moved into the crate `fps-tcp-own` (the adapter stays here), and TRN-20 added the smoltcp adapter `SmolStack`, see [The smoltcp adapter](#the-smoltcp-adapter) |
| [`fps-tcp-own`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-tcp-own) | `OwnStack`, `OwnSession`, `OwnConfig`, the modules `host` and `wire`, and the clocks `Clock`, `ManualClock`, and `MonotonicClock`, the own TCP stack of TRN-18 without any dependency on smoltcp | TRN-21 moved it out of `fps-tcp`, so an agent that runs the own stack does not link smoltcp, see [The own TCP stack](#the-own-tcp-stack) |
| [`fps-module-tls`](https://github.com/alrawi/FixingPointSystem/tree/main/modules/tls) | `TlsConnector`, `TlsAcceptor`, and `TlsSession`, which wrap any `SessionIo` in TLS 1.2 or 1.3, the client and server profiles, `RunAuthority`, and `NssKeyLog`, see [TLS module](#tls-module) | TRN-05 added the browser profiles, see [Browser profiles and JA4](#browser-profiles-and-ja4), and TRN-15 the mail and h2 profiles, see [Mail and h2 profiles](#mail-and-h2-profiles) |
| [`fps-backend-afxdp`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-backend-afxdp) | `FrameIo`, the frame trait with batched receive and send per queue, `open_port` with `AfXdpQueue`, `Workers`, the in memory `memory::frame_pair`, and `select_backend` of TRN-14, see [AF_XDP backend](#af_xdp-backend) and [Backend selection](#backend-selection) | TRN-12 runs a user space TCP stack over it, and TRN-19 measures its rate on KVM |

The `test-eng` target and the `rust` job of `ci.yml` run the tests of every crate under `engine/`, so the backend needs no make file or workflow of its own.

## Kernel TCP backend

The backend opens client sessions from host pool source addresses and accepts server sessions on pool addresses. Both sides return a `KernelStream`, which follows the four rules of the `SessionIo` seam.

### Client sessions

A `Connector` holds a `ConnectConfig` and opens one session per call to `connect(source, destination)`. Each session runs five steps.

1. Create the socket and apply the `SocketOptions`, so the receive buffer is in place before the handshake and the window scale reflects it.
2. Set `IP_BIND_ADDRESS_NO_PORT`, so `bind` takes the source address and leaves the port to `connect`.
3. Set `IP_LOCAL_PORT_RANGE` when the configuration names a `PortRange`.
4. Bind the source address with port 0.
5. Connect within `connect_timeout`, which defaults to 5 seconds.

Because the kernel picks the port at connect time, a source port is shared by every session whose 4-tuple differs. A port range of N ports therefore allows N concurrent sessions per source address and destination, not per source address. The loopback test `port_range_limits_sessions_per_destination_and_shares_ports_across_destinations` shows both halves of this rule. Without a `PortRange` the kernel uses `net.ipv4.ip_local_port_range` of the network namespace, which the HST-02 sysctl profile sets.

`IP_LOCAL_PORT_RANGE` needs Linux 6.3 or newer. An older kernel rejects it, and `connect` then fails with the step `set IP_LOCAL_PORT_RANGE` and the kind of `ENOPROTOOPT`. Ubuntu 24.04 runners in CI run a newer kernel. An exhausted range fails with `ErrorKind::AddrNotAvailable`, which statistics can count as an address error through `BackendError::io_kind`.

### Server sessions

`KernelListener::bind(address, &ListenConfig)` opens one listening socket, and `bind_pool(addresses, port, &ListenConfig)` opens one per pool address. The listener applies its settings in the order below.

1. `SO_REUSEADDR` when `reuse_address` is true, which is the default, so a listener rebinds while earlier sessions wait in TIME_WAIT.
2. `IPV6_V6ONLY` on IPv6 addresses, so IPv4 and IPv6 pool addresses share a port.
3. The `SocketOptions` of the configuration.
4. `listen` with `backlog`, which defaults to 4096. The kernel caps the value at `net.core.somaxconn`.

Accepted sessions inherit the buffer size, Nagle, and linger options from the listener, so an accept costs no extra system calls. The test `socket_options_reach_client_and_accepted_sockets` checks the inheritance on both sides.

### Socket options

`SocketOptions` configures clients and listeners with one type. TRN-03 added the fields `ttl`, `mss`, and `window_clamp` for the OS profiles described below.

| Field | Option | Default | Note |
|---|---|---|---|
| `send_buffer` | `SO_SNDBUF` | kernel default | Linux doubles the value and caps it at `net.core.wmem_max` |
| `recv_buffer` | `SO_RCVBUF` | kernel default | Linux doubles the value and caps it at `net.core.rmem_max` |
| `nodelay` | `TCP_NODELAY` | true | Nagle stays off, so the flow program controls segmentation |
| `linger` | `SO_LINGER` | `Linger::Default` | `Linger::Abort` sends a reset on every close |

`Linger::Timeout(d)` makes close block the calling thread until the kernel delivered unsent data or `d` passed, also on a nonblocking socket. A worker that drops such a session stalls, so we use it only in tests and at low rates.

### Abort and errors

`KernelStream::abort` sets `SO_LINGER` to zero and closes the socket. The kernel discards unsent data and sends a reset, and every later operation on the stream fails with `NotConnected`. The loopback tests check the reset on the peer for a client abort and for a server abort.

Every failure is a `BackendError` that names the source address, the destination or listening address, and the failed step, for example `connect from 127.11.0.7 to 127.10.0.1:40123: connect failed`. The operating system error stays available as the source of the error.

### Unsafe code

The crate denies `unsafe` code and allows it only in the private module `sys`. Neither `socket2` 0.6 nor Tokio exposes `IP_BIND_ADDRESS_NO_PORT` or `IP_LOCAL_PORT_RANGE`, and the `libc` crate does not define the constant of the second option yet. The module therefore calls `setsockopt` through `libc` with a 32 bit value, in one function with a `SAFETY` comment. Every other option goes through `socket2::SockRef`. When `socket2` gains both setters, we remove the module and return the crate to `forbid(unsafe_code)`.

## Tests

The component tests in `engine/fps-backend-kernel/tests/loopback.rs` use their own addresses in 127.0.0.0/8, which Linux routes to the loopback interface without setup. Each test takes a distinct /16, so TIME_WAIT entries of one test never collide with sessions of another. The table lists the tests.

| Test | Behavior |
|---|---|
| `ten_thousand_sessions_from_one_hundred_sources_without_address_errors` | The acceptance test of TRN-01. 10,000 echo sessions from 100 source addresses to two server pool addresses, 512 in flight, with the port range 40000 to 44999, finish without an error, and the servers see 100 sessions from each source |
| `port_range_limits_sessions_per_destination_and_shares_ports_across_destinations` | A four port range allows four sessions to one destination, the fifth fails with `AddrNotAvailable`, and the same four ports then serve a second destination |
| `client_abort_resets_the_server_and_disables_the_stream` and `server_abort_resets_the_client` | Abort sends a reset, and the aborted side reports `NotConnected` |
| `shutdown_sends_end_of_stream_and_keeps_the_other_direction_open` | Shutdown sends FIN and the reverse direction stays open |
| `socket_options_reach_client_and_accepted_sockets` | Buffer sizes, Nagle, and linger reach client sockets and accepted sockets |
| `refused_connect_names_source_destination_and_step` and `listener_reports_the_bound_address_in_bind_errors` | Errors carry addresses, steps, and the error kind |
| `ipv6_loopback_session_uses_the_configured_port_range` | The port range also applies to IPv6 sockets, and the test skips itself on hosts without IPv6 loopback |

The acceptance test finishes in about one second on a debug build. We run the tests with the command below.

```bash
cargo test -p fps-backend-kernel
```

## Limits

The backend leaves four concerns to other packages.

1. Pool addresses outside 127.0.0.0/8 need a local route or an address on an interface, which HST-02 sets up in a network namespace.
2. The choice of a source address per session belongs to the host pool allocator of HST-01. The connector takes the address that the caller passes.
3. TTL, MSS, window, SACK, and timestamp settings per operating system profile belong to TRN-03, which reaches the socket through `KernelStream::tcp_stream`.
4. Connect timeouts read the Tokio clock, which tests can pause, and the backend draws no randomness.

## Operating system profiles

TRN-03 added operating system profiles to the kernel backend. A profile names the TCP and IP behavior of one system, and test configurations select it by name in `os_profile`. The crate ships three profiles as YAML files in [`engine/fps-backend-kernel/profiles/`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-backend-kernel/profiles), and `OsProfile::builtin` embeds them in the binary. `OsProfile::from_yaml` loads a profile from other text and rejects unknown keys and values outside their range.

### Profile files

Each file has four sections. The backend applies `socket` to every socket, `system` lists the sysctls that Linux offers only per network namespace, and `reference` records the SYN of the real system for the realism check of RLM-09.

| Field | windows11 | linux6 | macos14 | Applied through |
|---|---|---|---|---|
| `socket.ttl` | 128 | 64 | 64 | `IP_TTL`, or `IPV6_UNICAST_HOPS` on IPv6 sockets |
| `socket.mss` | 1460 | 1460 | 1460 | `TCP_MAXSEG` before the handshake |
| `socket.window_clamp` | 16777215 | none | 3145728 | `TCP_WINDOW_CLAMP` before the handshake |
| `system.sack` | on | on | on | `net.ipv4.tcp_sack` |
| `system.timestamps` | off | on | on | `net.ipv4.tcp_timestamps` |
| `system.window_scaling` | on | on | on | `net.ipv4.tcp_window_scaling` |
| `system.tcp_rmem_max` | 16777216 | 6291456 | 4194304 | the maximum of `net.ipv4.tcp_rmem` |
| `reference.window_scale` | 8 | 7 | 6 | the result of the window clamp and `tcp_rmem_max` |
| `reference.syn_window` | 64240 | 64240 | 65535 | not settable, see the limits below |
| `reference.options` | mss, nop, ws, nop, nop, sok | mss, sok, ts, nop, ws | mss, nop, ws, nop, nop, ts, sok, eol | not settable, see the limits below |

`OsProfile::socket_options(base)` returns `base` with the TTL, MSS, and window clamp of the profile, so a caller keeps its own buffer, Nagle, and linger settings. `SocketOptions` therefore gained the three fields `ttl`, `mss`, and `window_clamp`, which default to `None` and extend the socket options table above. A listener passes all three to accepted sessions, because Linux copies them from the listening socket. `OsProfile::sysctls` returns the namespace settings as `sysctl -w` pairs for HST-02, and `OsProfile::sysctl_mismatches` with `read_proc_sysctl` reports every sysctl of the current namespace that differs from the profile.

Linux derives the window scale of the handshake as the base 2 logarithm of the smaller of the window clamp and the `tcp_rmem` maximum, less 15. The window clamp can therefore only lower the scale that the namespace allows, and Windows 11 reaches its scale of 8 only in a namespace with a `tcp_rmem` maximum of at least 8 MiB. The unit test `window_clamp_and_rmem_give_the_reference_window_scale` checks this rule against every profile.

`socket2` 0.6 has no setter for `TCP_WINDOW_CLAMP`, so the private module `sys` sets it through the existing `setsockopt` function. The module gained one more `unsafe` block, a `getsockopt` that only the unit tests compile, with its own `SAFETY` comment.

### Limits

The kernel backend cannot set the options below per socket. Each row names the control that Linux offers instead.

| Option | Why the backend cannot set it per socket | Linux control |
|---|---|---|
| TCP option order and padding | `tcp_options_write` writes a fixed order, for example mss, sok, ts, nop, ws, so no profile other than linux6 matches its reference order | none. The [own stack](#os-profiles-in-the-own-stack-trn-28) writes every order of the table |
| SACK permitted | system wide per network namespace | `net.ipv4.tcp_sack` |
| Timestamps | system wide per network namespace, and `TCP_TIMESTAMP` sets only an offset | `net.ipv4.tcp_timestamps` |
| Window scaling on or off | system wide per network namespace | `net.ipv4.tcp_window_scaling` |
| Window scale above the namespace limit | the clamp only lowers the scale, which the `tcp_rmem` maximum and `net.core.rmem_max` bound | `net.ipv4.tcp_rmem`, and `net.core.rmem_max`, which is global to the host |
| SYN window value | Linux rounds the window down to whole segments, so 65535 of macOS 14 appears as 64240 | `SO_RCVBUF` changes it only together with autotuning, and the route attribute `initrwnd` |
| Initial congestion window | per route | the route attribute `initcwnd` |
| ECN negotiation | per network namespace or per route | `net.ipv4.tcp_ecn` and the route feature `ecn` |
| Initial sequence numbers and IP identification | the kernel draws them | none |
| Retransmission and congestion control timing | `TCP_CONGESTION` selects only an algorithm of Linux | `net.ipv4.tcp_congestion_control` and the `rto_min` route attribute |

One network namespace holds one set of sysctls. Two profiles with different `system` sections, such as windows11 and linux6, therefore need separate namespaces, and HST-02 places each host pool in a namespace whose sysctls match its profile.

### Capture test

The capture test `engine/fps-backend-kernel/tests/netns/profiles.sh` runs on the [QA-01 harness](Testing.md#namespace-harness) with an MTU of 9000, so the route alone would allow an MSS of 8960. For each profile it writes the profile sysctls in the client and server namespaces, starts the example `os_profile_probe` as a server and as a client with that profile, and records the client arm. The helper `handshakes.py` then checks the SYN and the SYN-ACK. The SYN must carry the TTL and MSS of the profile, the SYN-ACK must carry the same MSS and a TTL one lower after the routing hop, and both must carry the window scale, SACK permitted, timestamp, and do not fragment settings of the profile. A last session without a profile must show TTL 64 and MSS 8960, which proves that the profiles and not the harness set the values. The test prints each handshake and notes where the option order differs from the reference.

```text
syn ttl=128 df=1 window=64240 mss=1460 wscale=8 options=mss,nop,nop,sok,nop,ws
syn-ack ttl=127 df=1 window=64240 mss=1460 wscale=8 options=mss,nop,nop,sok,nop,ws
syn ttl=64 df=1 window=64240 mss=1460 wscale=7 options=mss,sok,ts,nop,ws
syn ttl=64 df=1 window=64240 mss=1460 wscale=6 options=mss,sok,ts,nop,ws
syn ttl=64 df=1 window=62720 mss=8960 wscale=7 options=mss,sok,ts,nop,ws
```

The lines above are the SYNs of windows11 with its SYN-ACK, linux6, macos14, and the baseline session. The `capture` job of [`.github/workflows/trn.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/trn.yml) runs the test as root on `ubuntu-24.04` and keeps the captures as a workflow artifact, and the `test` job lints the helpers and runs their unit tests. We run the test locally with the commands below.

```bash
make build-trn-probe
sudo make e2e-trn               # pass TRN_PROBE=<target dir>/debug/examples/os_profile_probe for another target directory
make lint-trn test-trn          # shellcheck, ruff, mypy, and the unit tests of handshakes.py
cargo test -p fps-backend-kernel
```

The loopback test `os_profiles_reach_client_and_accepted_sockets` checks without root that every profile sets the TTL and MSS on client and accepted sockets.

#### Scoring with the transport check

TRN-16 made the test score every capture with `python3 -m realism score --os-profile NAME --client-hops 0 --server-hops 1` ([Realism](Realism.md#transport-behavior-check)). The runner reads the profile files in `engine/fps-backend-kernel/profiles/`, where `handshakes.py` reads the expected values from the probe, so the two checks are independent. Strict mode is on for linux6 and off for windows11 and macos14, whose option order the kernel backend cannot set (and macos14 also its SYN window), which stay in the report as deviations with `enforced: false`. The test reads the `transport_behavior` entry of each report and not the exit code of the runner, because the conformance check of the runner fails on the probe's own protocol, which nDPI does not know. A capture that gives no such entry fails too.

A capture recorded with the wrong profile fails. After the baseline session the test scores each capture with the next profile of the list (windows11 as linux6, linux6 as macos14, macos14 as windows11) and the baseline capture as linux6, and each score must give a failed `transport_behavior` entry. When the TTL of `windows11.yaml` is changed to 64 and the test runs again, the check of the windows11 capture fails with `the profile expects 64, the capture shows 128`. The runner needs `ndpiReader` (the conformance check always runs) and PyYAML (the profile files), and the test stops with a message when either is missing. With `FPS_KEEP_CAPTURES` the reports sit next to the captures.

**The window scale of the harness fixtures.** The module fixtures (`modules/*/fixtures/*.pcapng`) show window scale 10 where linux6 expects 7. The harness does not set `net.ipv4.tcp_rmem`, and a new network namespace copies the value of the host, which is `4096 131072 33554432` on the machine that we measured. The scale is the number of halvings that bring the buffer limit under 65536, which is 10 for 33554432 and 7 for the 6291456 of linux6. A fresh harness without a profile shows 10 in the SYN and the SYN-ACK. `net.core.rmem_max` plays no part, because it is 4194304 there and below the profile limit, so the namespaces allow the larger buffer through the inherited `tcp_rmem`. Sessions with a profile are not affected, because the test writes the `tcp_rmem` of the profile in both namespaces first, and every capture of the test shows the scale of its profile (8, 7, and 6). The baseline session shows 7 and not 10, because it runs after macos14 and keeps its 4194304. The fixtures stay as recorded, and `realism/examples/transport.labels.yaml` carries `expect: {window_scale: 10}` for them.

## Kernel UDP backend

TRN-02 added UDP sessions to `engine/fps-backend-kernel`. The type `KernelDatagram` implements the `DatagramIo` seam of [Engine](Engine.md#udp-datagram-sessions) over a kernel UDP socket that is connected to the session peer, on the client and on the server side, so every send goes to the peer and every receive returns only its datagrams.

### Client sessions

`Connector::connect_datagram(source, destination)` opens a client session with the same `ConnectConfig` as TCP. It creates the socket, applies the UDP subset of `SocketOptions`, which is the buffer sizes and the TTL, sets `IP_LOCAL_PORT_RANGE` when the configuration names a range, binds the pool source address with port 0, and connects to the destination. The kernel picks the source port at bind time, and a UDP port is not shared between destinations as a TCP port with `IP_BIND_ADDRESS_NO_PORT` is, so the range bounds the concurrent sessions per source address. Connecting a UDP socket sends nothing, so the connect timeout does not apply. An address family mismatch fails with `BackendError::AddressFamily` and every other step with `BackendError::Connect`, which names the step.

### Server sessions

`KernelDatagramListener::bind(address, options)` binds one pool address, and `bind_datagram_pool` binds one listener per address. `accept` waits for the first datagram of a peer without an open session. It then opens a second socket on the listener address and port with `SO_REUSEADDR` and `SO_REUSEPORT`, connects it to the peer, and returns it as a `KernelDatagram` with the first datagram queued in user space for the first receive, as the contract requires. Linux delivers every later datagram of the peer to the connected socket, because a connected socket outranks the unconnected listener in the lookup of a reuse port group. A failure while opening the session socket returns `BackendError::DatagramSession` with the peer and the step, and the listener stays usable. `accept` is cancel safe, so the open timeout of the interpreter loses no datagram.

Two races need care, and the backend handles both in user space.

1. A datagram of the peer can reach the listener after `accept` read the first one and before the session socket connects. The listener keeps a map from peer to the queue of each open session, and after every connect it drains its own socket without blocking, so such datagrams join their session at once. A datagram of a new peer that the drain reads waits in a read ahead queue of at most 1,024 datagrams for the next `accept`.
2. Between its bind and its connect a session socket is an unconnected member of the reuse port group, and Linux may deliver the first datagram of another peer to it. We saw this in the acceptance test, and a classic BPF reuse port program that always picks the listener does not prevent it once the group holds connected sockets. Server sessions therefore receive with the source address. A datagram of another peer is a stray, which the session hands back to its listener, and the listener opens a session for it on the next `accept`. The session drains its socket the same way right after the connect and again on close.

A datagram of a peer whose session is still open always joins that session. A client that reuses the source address and port of a session that the server still holds open therefore talks to the old session, because UDP has no close that would end it. The acceptance test keeps each client socket open until the next one is bound, so the kernel cannot hand out the same port twice in a row.

### Receives and errors

Receives pass `MSG_TRUNC`, so a datagram longer than the buffer keeps its first bytes and returns its full length, as `DatagramIo` requires. A server session with a buffer smaller than 64 KiB peeks at the source and length first, so it never cuts a stray short. The interpreter receives into a 64 KiB scratch buffer and takes the path with one system call.

A connected socket reports ICMP port unreachable as `ConnectionRefused`, which the interpreter maps to reason `connect_refused`. Linux signals that error with `EPOLLERR` alone, and the poll path of Tokio wakes a reader only on read readiness. A pending receive therefore also waits on `UdpSocket::ready(Interest::ERROR)`, which one boxed future per session holds. `try_io` takes a single interest, because Tokio masks a combined interest to an empty set.

`close` and drop release the socket, and every later operation fails with `NotConnected`. `send` rejects a datagram above 65,507 bytes with `InvalidInput` before it reaches the kernel.

### Interpreter openers

`DatagramConnectOpener` and `DatagramAcceptOpener` implement `fps_interp::DatagramOpener`. The connect opener binds one source address and takes the destination from the rendered host, which must be an IP address, or from its server address when the program names no host. `with_port` replaces the rendered port, as the port override of the runtime does for TCP. The accept opener borrows one listener, and the listener port takes the place of the rendered port. Both map a `BackendError` to an `io::Error` of its `io_kind`. The crate now depends on `fps-interp`, which needs nothing from the backend, and the tests depend on `fps-flow`.

### Unsafe code of the UDP path

The module `sys` gained one function. `socket2` receives into `&mut [MaybeUninit<u8>]`, so `sys::recv_from` casts the initialized receive buffer to that type under a `SAFETY` comment and passes `MSG_TRUNC`. No other UDP code uses `unsafe`.

### UDP tests

| Test | What it checks |
|---|---|
| `ten_thousand_request_response_pairs_from_one_hundred_sources` | 10,000 sessions with one request and one response each, from 100 source addresses in 127.21.0.0/24 to one listener, with 100 sessions in flight; every reply matches its request, the server sees all 100 sources, and every source port lies in the configured range |
| `one_session_carries_many_pairs_over_the_connected_server_socket` | 1,000 pairs over one session, so the server reads from its connected socket after the first datagram |
| `receives_truncate_and_return_the_full_length_on_both_sides` | `MSG_TRUNC` from the user space queue, the server socket, and the client socket, and a zero length datagram |
| `datagrams_that_race_the_session_connect_reach_the_session` | Five datagrams that wait before the first `accept` reach one session, a second peer gets its own session, each session hears only its peer, and a closed session gives way to a new one |
| `a_closed_port_reports_connection_refused` | ICMP port unreachable wakes a pending receive with `ConnectionRefused` |
| `close_and_oversized_sends_fail_with_their_kinds` | The 65,507 byte bound, a second `close`, and `NotConnected` after `close` |
| `address_family_mismatch_and_pool_binding` | The family check, `bind_datagram_pool`, and a second listener in the same reuse port group |
| `ttl_reaches_client_and_server_session_sockets` | The TTL of `SocketOptions` on client and server session sockets |
| `the_dns_lookup_example_runs_over_kernel_sockets` | The DNS lookup example of the contract through `Session::run_datagram_program` and both openers |

The file `tests/udp_loopback.rs` holds them, and the unit tests cover the queue bound of a session, the destination rules of the connect opener, and the UDP subset of `SocketOptions`. The acceptance test runs in about half a second, and it passed 50 repeated runs.

### UDP limits

| Limit | Reason |
|---|---|
| A reused five tuple joins a session that the server still holds open | UDP has no close, and the listener cannot tell a new client from the old one |
| A stray that reaches a session socket after its close drain is lost | The window between the kernel lookup and the enqueue is a few microseconds, and we found no case in testing |
| `MSG_TRUNC` and the stray handling work only on Linux | Other systems return only the copied bytes, which the crate does not target |
| The listener holds at most 1,024 datagrams of new peers and each session at most 64 | Datagrams beyond them are dropped, as a full kernel queue drops them |
| A server session socket replies from the listener address | A listener on the unspecified address would reply from the address of the route, so we bind specific pool addresses |

## TLS module

TRN-04 added the crate `fps-module-tls` in `modules/tls/native`. It wraps a `SessionIo` in TLS with rustls 0.23 and the ring crypto provider, the same provider and features as spike S2. A wrapped session is again a `SessionIo`, so a flow program runs over it unchanged, and `abort` resets the transport below the session. The workspace glob `modules/*/native` picks the crate up, and `cargo test --workspace` in the `rust` job of `ci.yml` runs its tests. The README at [`modules/tls/README.md`](https://github.com/alrawi/FixingPointSystem/blob/main/modules/tls/README.md) lists the layout and the commands.

### Profiles

A `ClientProfile` fixes what the ClientHello carries, and a `ServerProfile` fixes what the server accepts. Every field has a default, so `ClientProfile::default()` offers TLS 1.3 and 1.2 with the provider defaults.

| Field | Client | Server | Effect |
|---|---|---|---|
| `versions` | yes | yes | TLS 1.2, 1.3, or both |
| `send_sni` | yes | | The `server_name` extension. Rustls sends none for an IP address, whatever the flag says |
| `alpn` | yes | yes | Protocol ids in preference order. A server with a non empty list that shares none with a client that offers ids fails with `no_application_protocol` |
| `groups` | yes | yes | Key exchange groups in order, from `x25519`, `secp256r1`, and `secp384r1`. The ClientHello lists them in this order and the first one supplies the key share |
| `cipher_suites` | yes | yes | The nine suites of the ring provider in order. Suites of a version that is not enabled drop out |
| `sig_schemes` | yes | | The `signature_algorithms` extension in order, from the nine schemes of ring (TRN-05). It also limits the signatures that the client accepts |
| `use_server_cipher_order` | | yes | The server picks by its own order and not the client's |
| `resumption` | `Disabled` or `InMemory` | `Disabled`, `Tickets`, or `SessionCache` | See below |
| `key_log` | yes | yes | Receives the secrets of every handshake |

`CipherSuite`, `Group`, and `TlsVersion` implement `FromStr`, so a module can parse names from its settings. The test `client_hello_follows_the_profile` records the first bytes of a handshake, parses the ClientHello, and checks the order of suites, groups, ALPN ids, versions, the key share group, and the SNI against the profile. TRN-05 builds the browser profiles on these fields, see [Browser profiles and JA4](#browser-profiles-and-ja4).

### Browser profiles and JA4

TRN-05 added the module `fps_module_tls::browser` with four profiles. Each `BrowserProfile` carries the `ClientProfile`, the `expected_ja4` that this implementation puts on the wire, the `browser_ja4` of the real browser, the `sources` of its field orders, and the `gaps` that explain why the two fingerprints differ. `browser::BROWSER_PROFILES` lists them and `browser::find(name)` looks one up. The labels file of the realism check takes `expected_ja4` as the `ja4` of the profile `name` (see [Realism](Realism.md#encryption-check)), so RLM-04 reports the expected value for every profile.

**Rustls cannot reproduce a browser ClientHello, so no profile matches the JA4 of its browser.** The profiles follow the browsers where the crate has a knob, and the table records what comes out. The JA4 values below were computed from the bytes of a real handshake with the server name `tls.example.test` and cross checked with tshark 4.2.2.

| Profile | Follows | `expected_ja4` (this implementation) | `browser_ja4` (public captures) |
|---|---|---|---|
| `chrome-136` | Chrome 132 to 137 | `t13d1011h2_61a7ad8aa9b6_6d021c4c45cd` | `t13d1516h2_8daaf6152771_d8a2da3f94cd` |
| `edge-134` | Edge 122 to 134, a Chromium build | `t13d1011h2_61a7ad8aa9b6_6d021c4c45cd` | `t13d1516h2_8daaf6152771_02713d6af862` |
| `firefox-139` | Firefox 135 to 139 | `t13d1011h2_61a7ad8aa9b6_f674693fb0dc` | `t13d1717h2_5b57614c22b0_3cbfd9057e0d` |
| `safari-18.5` | Safari 18.2 to 18.5 on macOS | `t13d1011h2_61a7ad8aa9b6_6d021c4c45cd` | `t13d2014h2_a09f3c656075_e42f34c56612` |

What a profile controls, and what it follows from the browser:

| Field | Chrome and Edge | Firefox | Safari |
|---|---|---|---|
| Suite order (rustls subset, `0x00ff` appended) | 1301 1302 1303 c02b c02f c02c c030 cca9 cca8 | 1301 1303 1302 c02b c02f cca9 cca8 c02c c030 | 1301 1302 1303 c02c c02b cca9 c030 c02f cca8 |
| Signature schemes (ring subset) | 0403 0804 0401 0503 0805 0501 0806 0601 | 0403 0503 0807 0804 0805 0806 0401 0501 0601 | 0403 0804 0401 0503 0805 0501 0806 0601 |
| Groups (ring subset) | x25519, secp256r1, secp384r1 | the same | the same |
| ALPN | `h2`, `http/1.1` | the same | the same |

Four gaps hold for every profile. The first two decide the JA4 hash parts, so none of the four profiles can match.

1. **Cipher suites (JA4 part b and the count in part a).** Ring offers nine ECDHE AEAD suites, and rustls adds the signal suite `0x00ff`, so the ClientHello has 10 suites. A browser also offers the CBC suites `c009`, `c00a`, `c013`, `c014` (Safari also `c008`, `c012`, `000a`) and the RSA key exchange suites `009c`, `009d`, `002f`, `0035`, which no rustls provider implements. JA4 hashes the sorted suite list, so the hash is `61a7ad8aa9b6` for all four profiles, and the browsers' hashes are `8daaf6152771` (Chrome, Edge), `5b57614c22b0` (Firefox), and `a09f3c656075` (Safari). Order does not enter JA4, so the suite order above shows only in the wire form and in JA4_o.
2. **Extensions (JA4 part c and the count in part a).** Rustls sends 11 extensions, `0000`, `0005`, `000a`, `000b`, `000d`, `0010`, `0017`, `0023`, `002b`, `002d`, and `0033`. Chrome and Edge also send `0012` (signed_certificate_timestamp), `001b` (compress_certificate), `4469` or `44cd` (application_settings), `fe0d` (ECH GREASE), and `ff01` (renegotiation_info), Firefox also `0012`, `001b`, `001c` (record_size_limit), `0022` (delegated_credential), `fe0d`, and `ff01`, and Safari also `0012`, `001b`, and `ff01`. Rustls cannot send `0012`, `4469`, `44cd`, `001c`, `0022`, or `ff01` at all. `fe0d` needs an HPKE implementation, which the ring provider lacks (only aws-lc-rs has one, and the module follows decision D4 to keep ring). `001b` needs a certificate decompressor, and a stub that advertises compression without decoding would break handshakes with servers that compress certificates.
3. **GREASE.** Rustls adds no GREASE value to the suites, extensions, groups, or versions. JA4 drops GREASE before it counts and hashes, so it costs nothing in JA4, but it shows in JA3 and in the wire form.
4. **Extension order.** Rustls shuffles the extensions with a seed that it draws for each connection, and the crate cannot set it. Chrome and Edge shuffle too, Firefox and Safari use a fixed order. JA4 sorts the extensions, so it is stable, and JA4_o and JA3 are not.

The profiles also differ from their browsers in groups (no `X25519MLKEM768`, P-521, or FFDHE) and in signature schemes (no SHA-1 schemes, P-521, or Ed448), which JA4 does not read for groups and reads for schemes in part c.

Because only the signature scheme order differs, three of the four profiles share one fingerprint. Chrome, Edge, and Safari list the same schemes in the same order after the filter, so `chrome-136`, `edge-134`, and `safari-18.5` have the same `expected_ja4`, and only `firefox-139` differs, in part c. A fingerprint check cannot tell these three profiles apart, so a realism run that needs to separate Chrome from Safari traffic has to use the HTTP layer (the `User-Agent` and header order) and not JA4. This is the honest limit of rustls. Matching the browsers needs a TLS stack that can set the suite list and the extensions freely, for example BoringSSL, whose license needs a review against GPL-3.0 first, or a patched rustls that takes a ClientHello hook. Either needs an owner decision, and this package does not make it.

**Sources.** The `browser_ja4` values and the version ranges come from the expectations in the tests of `wreq-util` 2.2.6, which check the emulations of Chrome, Edge, Firefox, and Safari against captured fingerprints ([Chrome](https://docs.rs/crate/wreq-util/2.2.6/source/tests/emulation_chrome.rs), [Firefox](https://docs.rs/crate/wreq-util/2.2.6/source/tests/emulation_firefox.rs), [Safari](https://docs.rs/crate/wreq-util/2.2.6/source/tests/emulation_safari.rs)). The Chrome fingerprint of the FoxIO worked example, `t13d1516h2_8daaf6152771_e5627efa2ab1`, comes from the [JA4 specification](https://github.com/FoxIO-LLC/ja4/blob/main/technical_details/JA4.md), and its ClientHello is the fixture of `realism/tests/test_ja4.py`, which is the source of the Chrome suite and signature scheme order. The Firefox and Safari suite orders and signature scheme orders are the orders of public ClientHello captures (tls.peet.ws, ja4db) as the author recalled them, and the author could not fetch the capture pages in the sandbox of this package. Treat those two rows as unverified until someone compares them with a capture of the named version. The gap on this point does not change `expected_ja4` for Firefox or Safari, because JA4 ignores the suite order, but it would change the Firefox value if the scheme order were wrong. The names `chrome-136`, `edge-134`, `firefox-139`, and `safari-18.5` name the newest version of each range in the sources, which is older than the stable releases of November 2026, because no newer capture was available.

**Tests.** `modules/tls/tests/ja4.rs` runs a real handshake per profile, records the ClientHello bytes, and computes the JA4 with `realism/ja4.py` through `modules/tls/tests/ja4_of_hello.py`, which needs only `python3`.

| Test | Needs | Check |
|---|---|---|
| `every_browser_profile_produces_its_expected_ja4` | `python3` | The JA4 of the recorded ClientHello equals `expected_ja4` and differs from `browser_ja4` |
| `profile_orders_reach_the_wire` | `python3` | The suites (plus `00ff`), the signature schemes, and the ALPN ids appear in the order of the profile |
| `profiles_that_share_a_client_hello_share_a_ja4` | none | Chrome and Edge have one `expected_ja4` and two `browser_ja4` values |
| `tshark_agrees_with_the_expected_ja4` | `tshark` 4.2 or newer | `tls.handshake.ja4` of tshark equals `expected_ja4` for every profile |

The Python tests run in the `rust` job of `ci.yml` through `cargo test --workspace`. The `mod-tls` workflow runs only `--test capture`, so the tshark test does not run there until the workflow has the step `cargo test -p fps-module-tls --test ja4` with `FPS_REQUIRE_CAPTURE_TOOLS: "1"`, see the log entry of TRN-05.

### Mail and h2 profiles

TRN-15 added `fps_module_tls::mail` with three profiles whose names are part of the interface with the agent.

| Name | Side | ALPN | Result |
|---|---|---|---|
| `mail-client` | client | none | The ClientHello has no ALPN extension. Suites, groups, signature schemes, and versions are those of `firefox-139`, because Thunderbird shares its TLS library, so the profile differs from `firefox-139` in the ALPN list alone and has the gaps of [Browser profiles and JA4](#browser-profiles-and-ja4) |
| `server-tls13-h2` | server | `h2`, `http/1.1` | A client that offers `h2` gets it, a client that offers only `http/1.1` gets that, and a client without ALPN gets none |
| `server-tls13-noalpn` | server | none | The handshake of a client that offers ids completes and selects none, so the report holds no ALPN |

Both servers accept TLS 1.3 only and issue no tickets, as `server-tls13` of the agent does. A caller that builds one connector per session sets `resumption` on the value that `mail::server_tls13_h2` or `mail::server_profile` returns. The tests of `modules/tls/tests/burst.rs` handshake six pairings of these profiles with `chrome-136`, `firefox-139`, `mail-client`, and an `http/1.1` only client, check the ALPN of both reports, and parse the ClientHello of `mail-client` to show that it carries no ALPN extension.

### Certificates

`RunAuthority::generate` makes an ECDSA P-256 authority with a fresh key, and `issue(names, LeafKey)` signs a server certificate for DNS names with the key type `EcdsaP256`, `EcdsaP384`, `Ed25519`, or `RsaPem`. The ring provider cannot generate RSA keys, so `RsaPem` takes a PKCS#8 PEM key that the caller made, for example with `openssl genpkey`, as spike S2 does. The chain holds the leaf and the authority. `root_store` returns a store that trusts only the authority of the run, so a client verifies the server as a browser would, and a client of another run fails with `unknown_ca`. Key generation draws from the system random source, which no run seed controls, so certificates differ between runs by design.

### Sessions and reports

`TlsConnector::connect(io, server_name)` and `TlsAcceptor::accept(io)` run the handshake and return the session. Each session carries a `HandshakeReport` with the side, version, IANA cipher suite name, negotiated group, ALPN id, SNI, whether the handshake resumed, and its duration on the Tokio clock. These values feed `tls.handshakes`, `tls.resumptions`, and `tls.handshake_time`, and the `side` of the report holds the values of the catalog's `side` label.

A failure is a `TlsError::Handshake` that names the side and peer, keeps the rustls error behind `source`, and carries the alerts as the lower snake case names of the `alert` label: `alert_received` for `tls.alerts.rx` and `alert_sent` for `tls.alerts.tx`. `failure_reason` returns `tls_failure`. Rustls sends some alerts encrypted, so `alert_sent` comes from the plaintext record when there is one and from the error otherwise, and it covers `unknown_ca`, `bad_certificate`, `certificate_expired`, `handshake_failure`, `no_application_protocol`, and `bad_record_mac`. The module does not depend on `fps-stats`. The interpreter or runtime that owns the counters calls `TransportCounters::tls_handshake`, `tls_resumption`, and `tls_handshake_failure` from the report and the error.

The session wrapper is written directly over `rustls::Connection`, so the module needs no `tokio-rustls`. Five behaviors follow from it.

1. The wrapper flushes pending records before it waits for more ciphertext, so a key update or an alert never waits behind a read.
2. The last handshake flight goes out in full, except the two session tickets of a TLS 1.3 server, which go out as far as the transport takes them. The next read, write, flush, or shutdown sends the rest. Waiting for them deadlocks two peers over a transport whose buffer is smaller than the tickets, and the test `large_transfers_survive_one_byte_reads_and_back_pressure` found this with a 37 byte pipe.
3. A TLS 1.3 client stores its tickets when it reads them, which is with its next read after the handshake. A client that closes right after the handshake therefore does not resume next time.
4. A transport that ends without `close_notify` reads as end of stream, because many servers close that way. `shutdown` sends `close_notify` and then shuts the transport down.
5. A full plaintext buffer is backpressure (TRN-15). Rustls holds at most 16 KiB of received plaintext and refuses more ciphertext with an `ErrorKind::Other` error until the application reads. A ciphertext chunk of 16.5 KiB can complete more than that, for example a full 16 KiB record followed by small records, as HTTP/2 frames produce them, and the wrapper used to fail the read with "received plaintext buffer full" (MOD-06). It now keeps the part of the chunk that rustls refused in its read buffer, returns the plaintext first, and feeds the rest before it reads the transport again. The tests `bursts_of_full_and_tiny_records_survive_a_slow_reader`, `frame_shaped_bursts_survive_a_slow_reader`, and `a_burst_that_ends_with_close_notify_reads_to_the_end` failed with that error before the change, and `one_mebibyte_in_one_burst_both_ways` moves 1 MiB each way with both writers finished before either reader starts, on TLS 1.3 and 1.2. A handshake that meets a full buffer fails, because no plaintext exists before it ends.

### Resumption

A connector owns its resumption store, so every session of one connector to the same server name can resume, and a run shares one connector per client profile. The test `resumption_follows_the_profiles_on_both_versions` shows the first handshake of a series as full and the later ones as resumed, with server tickets and with a server session cache on both versions, and shows no resumption when the client store or the server policy is off. A new connector starts with an empty store.

### Key log

`NssKeyLog` implements the rustls `KeyLog` trait and writes one line per secret in the NSS format `LABEL <client random> <secret>`, in memory or appended to a file with one write per line. `NssKeyLog::from_env` reads `SSLKEYLOGFILE` for the edge of a process. TLS 1.3 handshakes write `CLIENT_HANDSHAKE_TRAFFIC_SECRET`, `SERVER_HANDSHAKE_TRAFFIC_SECRET`, `CLIENT_TRAFFIC_SECRET_0`, `SERVER_TRAFFIC_SECRET_0`, and `EXPORTER_SECRET`, and TLS 1.2 handshakes write `CLIENT_RANDOM`. The test `key_log_lines_match_the_handshake` checks that every line names the ClientHello random of the recorded handshake. A key log file holds the keys of every session it saw, so treat it like a private key and keep it out of commits.

### Capture and decryption tests

`modules/tls/tests/capture.rs` runs one HTTP exchange over TLS 1.3 and one over TLS 1.2 on in memory sessions with a recording wrapper, writes the bytes as a classic pcap of one TCP connection to port 443 with a three way handshake, and writes the key log next to it in the target temp directory of cargo. Two tests then check the keys.

| Test | Needs | Check |
|---|---|---|
| `tshark_decrypts_captures_with_exported_keys` | `tshark` 3.0 or newer | `tshark -r <pcap> -o tls.keylog_file:<keys> -Y http.request -T fields -e http.request.uri` prints `/fps-tls-sample`, the response filter prints `200`, and the same command without the key log prints nothing |
| `python_decrypts_a_tls13_capture_with_exported_keys` | `python3` with `cryptography` | `tests/decrypt_tls13.py` takes the ClientHello random from the capture, looks up its secrets in the key log, derives the record keys as RFC 8446 section 7.3 describes, and decrypts both directions. A key log with one flipped digit per secret decrypts nothing |

The Python test passed in the sandbox of this package, so the key log format and the capture bytes work with an implementation that shares no code with rustls. The tshark test could not run there because tshark is not installed, and it skips itself where tshark is missing. A job that has tshark runs it with `FPS_REQUIRE_CAPTURE_TOOLS=1 cargo test -p fps-module-tls --test capture`, which turns a missing tool into a failure. The `realism` workflow already installs tshark through `realism/ci/install-tools.sh`, so that command as a step there, or in a job that runs `apt-get install tshark`, completes the check. Both files live outside the paths of TRN-04.

### Handshake rates

The benchmark `handshake_bench` repeats the method of spike S2. A client and a server run in memory on one thread, the calls of each side are timed separately, the server presents a leaf and authority chain, the client verifies it, TLS 1.3 handshakes include the delivery of the tickets, and every profile of S2 runs. The only difference from the spike is that the rustls configurations come from the module's profiles. We ran the spike and the module in alternation, three rounds of 3 seconds per profile, on the same machine, so that drift of the shared sandbox hits both sides. The table lists handshakes per second on one core for both sides together, as the median of the three rounds. The phase 0 column holds the figures of `spikes/s2-tls/RESULTS.md`, which came from another build on the same machine type.

| TLS | Leaf key | Group | Mode | Phase 0 | Spike now | Module | Module / spike |
|---|---|---|---|---|---|---|---|
| 1.3 | ECDSA P-256 | X25519 | Full | 2,390 | 2,762 | 2,861 | 1.036 |
| 1.3 | ECDSA P-256 | X25519 | Resumed | 4,404 | 4,606 | 4,865 | 1.056 |
| 1.2 | ECDSA P-256 | X25519 | Full | 2,976 | 3,055 | 3,192 | 1.045 |
| 1.2 | ECDSA P-256 | X25519 | Resumed | 33,459 | 33,464 | 33,449 | 1.000 |
| 1.3 | ECDSA P-256 | secp256r1 | Full | 2,761 | 2,622 | 2,630 | 1.003 |
| 1.3 | RSA 2048 | X25519 | Full | 1,080 | 1,078 | 1,086 | 1.007 |
| 1.3 | RSA 2048 | X25519 | Resumed | 4,407 | 4,804 | 4,812 | 1.002 |
| 1.3 | RSA 2048 | secp256r1 | Full | 1,078 | 1,100 | 1,053 | 0.958 |
| 1.2 | RSA 2048 | X25519 | Full | 997 | 1,158 | 1,079 | 0.932 |
| 1.2 | RSA 2048 | X25519 | Resumed | 30,078 | 32,212 | 31,553 | 0.980 |

Over all 30 client, server, and both sides figures, the lowest median ratio to the spike is 0.922, for the client side of TLS 1.2 with RSA, and 24 of the 30 ratios are 0.98 or higher. Against the phase 0 figures every one of the 30 medians lies at 0.95 or higher, and the lowest is TLS 1.3 with ECDSA and secp256r1. The acceptance limit of 0.90 therefore holds in this sandbox, but the sandbox is noisy. It has two shared vCPUs, other builds ran beside the benchmark, and the spike alone varies by 6% between its own rounds, for example from 5,020 to 5,707 client handshakes per second for TLS 1.2 with RSA. The ratio of the medians is the figure to trust and not a single run, and the sandbox cannot show a loss smaller than that noise. The owner host can repeat the comparison with `modules/tls/bench/run_s2.sh`, which builds both binaries, alternates them, and exits with status 1 when a median ratio falls below 0.90. The raw files of this run are in `modules/tls/results/phase2-sandbox/`.

The async wrapper costs more than the profile code. `handshake_bench --mode session` runs the full `TlsConnector` and `TlsAcceptor` over an in memory `SessionIo` pair on a current thread runtime, with one byte from the server after each handshake so that TLS 1.3 clients read their tickets, and reports the rate of both sides together. One run in the sandbox gave 2,715 for TLS 1.3 full with ECDSA (5% below the in memory method), 2,713 for TLS 1.2 full with ECDSA (15% below), 4,691 for TLS 1.3 resumed (4% below), and 28,703 for TLS 1.2 resumed with ECDSA (14% below). The difference covers the allocation of the pipe and the buffers, the task switches of the runtime, and the application byte. A single run is noisy, so treat these as an upper bound of the wrapper cost. The wrapper matters most for resumed TLS 1.2 handshakes, which cost about 30 microseconds, while a full handshake costs 300 to 900 microseconds and a kernel TCP session adds more than the wrapper does.

### Limits

| Limit | Reason |
|---|---|
| No client certificates, no 0-RTT, and no custom certificate verifier | The first version verifies the server against the run authority and offers no mutual TLS. A package that needs them extends the profiles |
| No post quantum groups, AES-CCM, or CBC suites | The ring provider does not implement them |
| The ClientHello of a profile cannot match a browser's JA4 | Rustls with ring fixes the extension set, shuffles the order, adds no GREASE, and offers no CBC or RSA key exchange suites, see [Browser profiles and JA4](#browser-profiles-and-ja4) |
| A TLS 1.3 server flushes its tickets lazily | See the session behaviors above |
| A transport that ends without `close_notify` reads as a clean end | Truncation is not an error, which suits servers that close this way |
| `alert_sent` is known only for plaintext alerts and for errors that imply one | Rustls sends the other alerts encrypted and does not report them |
| Certificate and key generation use the system random source | No run seed controls them |

## AF_XDP backend

TRN-11 added the crate `fps-backend-afxdp`. It gives a user space transport whole Ethernet frames in batches on one queue of a network device, and it does not parse a frame. TRN-12 builds the user space TCP stack on top of it, and HST-03 answers ARP and neighbor discovery over it. The crate depends only on `libc` and `thiserror`, needs Linux 5.9 or newer for the BPF link that attaches its program, and needs root or the capabilities `CAP_NET_ADMIN`, `CAP_NET_RAW`, and `CAP_BPF`. The tests ran only as root, so the capability list is untested.

### The frame trait

`FrameIo` is the seam between a transport and a device. A queue belongs to one thread, so no method takes a lock, and no method blocks except `wait_readable`.

| Method | Behavior |
|---|---|
| `recv_batch(max, frame)` | Calls `frame` once for each of at most `max` received frames and returns the count. The slice lives only inside the call, and the queue recycles the memory after the batch, so a batch costs no allocation |
| `send_batch_with(count, fill)` | Lends up to `count` transmit buffers of `max_frame_len` bytes, calls `fill(index, buffer)` for each, and queues the frame of the length that `fill` returns. A return value of 0 ends the batch. The call returns the number queued, which is lower than `count` when the queue has no free frame or slot |
| `send_batch(frames)` | Copies each frame into queue memory and rejects an empty or oversized frame before it queues any |
| `flush()` | Wakes the kernel to send and takes back the memory of sent frames. A worker with nothing to receive calls it |
| `wait_readable(timeout)` | Sleeps until a frame arrives or the timeout passes |
| `stats()` | The counters of the queue, such as frames, bytes, full sends, and kicks |

The crate keeps the trait here and not in `fps-io`, because the catalog paths of TRN-11 exclude `engine/fps-io`. A DPDK backend can implement the same trait, and `memory::frame_pair` gives tests of the code above it a fake with a bounded depth, so a full link makes a send return short as a full transmit ring does.

### Opening a port

`open_port(&PortConfig)` opens one socket per queue, attaches the XDP program, and returns an `OpenedPort` with a `PortInfo` and one `AfXdpQueue` per queue in the order of `PortConfig::queues`. It runs five steps.

1. Look up the interface index and create an XSK map with one slot per queue number.
2. For each queue, map a UMEM, register it, size and map the four rings, fill the fill ring with the whole receive pool, and bind with the requested mode.
3. Read the bind mode back from the socket with `getsockopt(XDP_OPTIONS)` and store the socket in the map.
4. Load the program, which has six BPF instructions. It sends a frame of queue `n` to the socket at slot `n` and passes the frame to the kernel stack when the map has no socket for that queue, so queues that the port does not open keep working.
5. Attach the program through a BPF link, in the driver when the device supports it and in the generic hook after an `EOPNOTSUPP`. `PortInfo::xdp_mode` says which.

The link detaches when the last queue of the port drops, so no program outlives the process. A second port on an interface that already has an XDP program fails with `EBUSY`, at the `bind` step when a queue is taken and at the step `attach XDP program` otherwise. Every failure is an `AfXdpError` that names the step, the interface, and the queue, and keeps the operating system error as its source.

### Bind modes and the copy fallback

`BindMode::Auto` binds without a mode flag, so the kernel tries zero copy and falls back to copy mode when the driver cannot do it. `BindMode::ZeroCopy` fails at the step `bind` on such a driver, and `AfXdpError::is_zero_copy_unsupported` recognizes that failure. `BindMode::Copy` forces copy mode. The request never says what runs, so `PortInfo::queues` and `AfXdpQueue::bind_mode` return the `BoundMode` that the socket reports through `XDP_OPTIONS`. A veth pair has no zero copy driver, so `Auto` reports `Copy` there and `ZeroCopy` fails with `EOPNOTSUPP` (errno 95).

### Queue memory

Each queue owns its UMEM, so queues share no memory. The UMEM holds `2 * ring_size` frames of `frame_size` bytes in aligned chunk mode. The first half is the receive pool. It starts in the fill ring, comes back through the receive ring, and returns to the fill ring after `recv_batch` called the callback. The second half is the transmit pool. A frame leaves a free list for the transmit ring and returns from the completion ring. The rings are single producer and single consumer queues of indices that only grow and wrap, with release stores to publish and acquire loads to read, and a unit test moves one million entries between two threads. A frame that the kernel returns with an address outside the pool is counted in `rx_invalid` and dropped, and the crate never panics on kernel data.

The kernel asks for wake ups through a ring flag (`XDP_USE_NEED_WAKEUP`). In zero copy mode the queue calls `sendto` or `recvfrom` only when the flag is set. In copy mode the kernel sends inside `sendto`, at most 32 frames per call, and answers `EAGAIN` while more wait, so `send_batch_with` repeats the call until the transmit ring is empty. A frame must fit in `frame_size - 256` bytes, because the kernel keeps `XDP_PACKET_HEADROOM` in front of a received frame, and `max_frame_len` is that number for both directions.

### Workers

`Workers::spawn(queues, &WorkerOptions, make)` starts one thread per queue and calls `make(queue_id)` for the `QueueLoop` of each. A closure `FnMut(&mut dyn FrameIo) -> io::Result<bool>` is a loop, and it returns whether it moved a frame. `WorkerOptions` pins worker `i` to `cpus[i % cpus.len()]` and picks what an idle worker does with `Idle`, which is `Spin`, `Yield`, or `Wait(duration)` in `wait_readable`. The default is a 1 ms wait without pinning, and a rate test uses `Spin`. `Workers::stop` returns one `WorkerReport` per queue with the turn counts, the final counters, and the error that ended the worker early. A loop that returns an error ends only its own worker. Every worker takes at least one turn, even when `stop` comes before its thread starts, so the error of a broken queue is always reported (bug #588). Workers are generic over `FrameIo`, so the same code runs over `memory::frame_pair` in unit tests.

### Defaults from spike S3

`QueueConfig::default` takes its values from `spikes/s3-afxdp/RESULTS.md` and from the logs of that run.

| Setting | Default | Reason |
|---|---|---|
| `batch` | 32 | The `testpmd` runs of S3 forwarded 32 packets per burst, as their logs show |
| Queues per worker | one | S3 reached 10 Gbps of 1,500 byte frames with one or two queues of a three vCPU guest, one core each |
| `mode` | `Auto` | Default mode did not beat forced copy mode beyond the noise of the host, so zero copy is a gain where a driver offers it and costs nothing where it does not |
| `frame_size` | 2048 | The chunk leaves 1792 bytes after the kernel headroom, which holds a 1500 byte MTU |
| `ring_size` | 2048 | A choice. S3 did not vary the ring size, and TRN-19 tunes it |

S3 ran on a small host, so its results are indicative. Its `zero_copy_log_lines` column counts a DPDK buffer message and not the bind mode of the kernel, which is why this crate reads the mode from the socket.

### Backend selection

`select_backend(&SelectionInput)` in `fps-backend-afxdp` (TRN-14) picks the transport backend at boot and says why. `HostCapabilities` lives in `fps-agent`, which depends on the backends, so the function takes an input type of its own and AGT-22 builds it from the detection and reports the result in `Capabilities.selected_backend`. The module has no dependency and builds on every platform.

| Input | Meaning |
|---|---|
| `forced` | `Some(Kernel)` or `Some(AfXdp)` forces a backend, and `None` lets the rules choose |
| `kernel.transport_available` | The kernel backend carries sessions, which `fps_backend_kernel::TRANSPORT_AVAILABLE` says is true everywhere |
| `af_xdp.kernel_support` | The running kernel offers AF_XDP sockets |
| `af_xdp.transport_available` | A transport carries TCP over AF_XDP. `fps_backend_afxdp::TRANSPORT_AVAILABLE` is false until TRN-12 sets it |
| `af_xdp.driver` | The best XDP mode that every port offers: `None`, `Generic`, `Native` (copy mode, as a virtio-net guest without zero copy), or `ZeroCopy`. The values match the contract `XdpMode` |

The rules, in order:

- The kernel backend is the default, and the result then carries the reason AF_XDP lost (`Unavailable`).
- AF_XDP wins when the kernel offers it, a transport exists, and the driver offers `Native` or `ZeroCopy`. The result carries the expected mode.
- `Generic` never wins by itself, but an operator can force it. This is a policy, not a measurement: spike S3 measured only native copy and zero copy, so TRN-12 revisits the rule with its own numbers.
- A forced backend that cannot run returns `SelectError::ForcedUnsupported` with the cause and never falls back, so a benchmark does not measure the wrong backend. With nothing forced and no working backend the error is `SelectError::NoBackend`.

Every host selects the kernel backend until TRN-31 sets `TRANSPORT_AVAILABLE`, so AF_XDP is never the default today. An operator who forces AF_XDP gets it on a host whose kernel and driver can run it (AGT-24): the agent sets `transport_available` for a forced run, because the datapath of TRN-27 is a transport, and the constant only keeps AF_XDP from becoming the default before the own stack has loss recovery and congestion control. A forced run on a host without AF_XDP sockets or without a driver that takes XDP is still refused with the cause. The tests in `tests/select.rs` cover a virtio-net guest with and without zero copy, an old kernel, a generic only driver, a driver without XDP, and forced backends. The expected bind mode is a plan, not a measurement: a run that starts the AF_XDP backend reads the mode that each socket really runs in (`BoundPort`, [Agent](Agent.md#starting-the-selected-backend)). A forced AF_XDP run runs the http1 and dns modules on the datapath through the agent (AGT-25): the agent gives every worker `fps_agent::transport::DatapathTransport`, an implementation of `fps_runtime::transport::Transport` (ENG-31) that listens once for each port on the datapath and routes the accepted sessions by their local address to the server worker that holds the host, and it maps the `os_profile` of each host pool to the OS profile of its side in the own stack. When a host pool of the plan carries a gateway, the agent turns on the neighbor resolution of the datapath, so the peers behind that gateway need no neighbor entry (AGT-26). The agent runs IPv4 or IPv6 host pools on AF_XDP, one family for both arms (AGT-28); a pool with both prefixes runs over IPv4. It still refuses DHCP host pools there until AGT-23 uses the lease phase of the datapath (TRN-37).

### Tests

The unit tests need no privilege. They cover the settings, the rings (order, wrap, full, index overflow, the wake up flag, and two threads), the system call layer (structure sizes, the BPF instructions, mappings, and the CPU pin), the in memory pair, and the workers over it.

`engine/fps-backend-afxdp/tests/veth.rs` is the acceptance test of TRN-11. It brings up the [QA-01 harness](Testing.md#namespace-harness) and runs helper processes, which are the test binary again, in the client and router namespaces with `ip netns exec`. It has three parts.

| Part | Result |
|---|---|
| Modes on `eth0` | `ZeroCopy` fails with `EOPNOTSUPP`, a ring size of 100 is refused before any system call, `Copy` and `Auto` bind in copy mode and the socket confirms it, and a second port on the interface fails with `EBUSY` |
| One queue | The client sends 4,000 frames of 60 to 1,514 bytes from `eth0`, the router echoes them from `cli0` with the MAC addresses swapped, and all 4,000 return with their payloads intact |
| Two queues | The same over a veth pair with two queues, `xq0` and `xq1`, with one `Workers` thread per queue on both ends. Each frame returns on the queue that sent it, and the router echoes exactly 4,000 frames on each queue. One queue sends with `send_batch` and the other with `send_batch_with` |

The test needs root. As a normal user it runs itself again with `sudo -n env FPS_AFXDP_UNDER_SUDO=1`, and it skips with a message when neither works. `CI=true` or `FPS_REQUIRE_AFXDP=1` turn a skip into a failure, so a runner that cannot run it does not pass silently. `FPS_AFXDP_FRAMES` sets the frames per queue, and a run with 100,000 frames per queue in the sandbox of the package passed. The test runs inside `cargo test -p fps-backend-afxdp`, and so inside the `cargo test --workspace` step of the `rust` job of `ci.yml`.

```bash
cargo test -p fps-backend-afxdp                          # unit tests and the veth test, which uses sudo
FPS_AFXDP_FRAMES=100000 cargo test -p fps-backend-afxdp --test veth -- --nocapture
```

### Limits

| Limit | Reason |
|---|---|
| The zero copy bind has no test | The sandbox and CI have only veth pairs, which have no zero copy driver. TRN-19 binds in zero copy mode on virtio-net guests |
| One XDP program per interface | The kernel allows one program on a device. A port that meets another program fails with `EBUSY` and does not replace it |
| A closed queue stays busy for a moment | The kernel releases the queue of a closed socket in a work queue, so a bind right after a close can fail with `EBUSY` for a few milliseconds, and the test retries for up to five seconds |
| No shared UMEM and no unaligned chunks | One UMEM per queue keeps workers independent, and the aligned mode needs a power of two frame size from 2048 bytes up to the page size |
| No busy polling | S3 ran DPDK with a busy poll budget of 64, and TRN-19 decides whether this crate needs `SO_PREFER_BUSY_POLL` |
| Linux on 64 bit only | The ring offsets and the `bpf` call use the 64 bit layout, and other targets get only the trait, the settings, the workers, and the in memory pair |

## TCP stack bench harness

TRN-17 added the crate `engine/fps-tcp`, which measures a TCP stack behind `SessionIo` for decision record 0014 of TRN-10. A stack implements `TcpStack`, which models a client host and a server host, and every session leaves the stack as a `SessionIo`, so a measurement includes the cost of the adapter. The kernel loopback TCP is the reference stack of TRN-17. TRN-18 plugged an own stack into the same trait, TRN-20 plugged smoltcp in, and TRN-13 can reuse the workloads for the DPDK path.

The three stacks coexist behind the seam, and `tcp_bench --stack kernel|own|smoltcp` picks one. smoltcp and `managed` are licensed 0BSD, which `deny.toml` allows since the owner decision of 2026-11-17, so the harness has the dependency `smoltcp 0.14` and `cargo deny check` passes.

### Workloads

The three workloads read the wall clock because they measure it, and they run on a current thread runtime with a driver task that polls a user space stack, like a poll mode worker. A stack that the operating system drives has a driver that only waits.

| Workload | What it does | What it reports |
|---|---|---|
| `session_rate` | `concurrency` tasks open a session, send a request, read a response, close the write side, and wait for the end of the server, until the duration passes | Completed sessions per second, failed sessions, and the p50 and p99 time of a session |
| `throughput` | `sessions` concurrent sessions move `bytes_per_session` each, as upload or download, in chunks of 64 KiB, and the workload checks that every byte arrived | Delivered application Gbit per second |
| `socket_scaling` | Opens sessions in batches until a step reaches its socket count, in steps such as 1,000 up to 1,000,000, spread over shards | Per step the setup rate, the resident memory per socket, the time of one full poll with idle sockets, and the session rate and p99 that a probe sees while all sockets are held |

A socket count includes both hosts of a pair, so 1,000,000 sockets are 500,000 sessions, and it includes the few listening sockets. A step that exceeds its time budget, or that the stack refuses a socket for, such as at the open files limit, ends the run and is marked `not reached` with the reason on a `stopped` line, so a stack that slows down with the socket count stops the run in place of hanging it. A shard is an independent pair, with its own interfaces and socket sets, which is how an engine worker owns its sessions. At most 20,000 sessions share one server port, which keeps every 4-tuple unique within the 28,000 source ports of a Linux client. The scaling workload runs all shards on one thread, so the memory and poll numbers do not depend on thread count. The rate and throughput workloads run one stack per thread with `--threads`.

### The kernel reference stack

`KernelStack` is the TCP of the operating system on the loopback interface. The client host and the server host are sockets of one kernel that talk over `127.0.0.1`, driven by the reactor of the tokio runtime that the workload runs on, so the stack is the baseline that a user space stack must match or beat. It follows three rules, which `engine/fps-tcp/src/kernel.rs` documents in full.

1. The stack needs no driver. The kernel moves the segments and runs the timers, so `drive` only waits and `poll_once` does nothing, and the idle poll cost of a kernel run is zero by construction.
2. A listened port is a logical number of the harness. Each stack binds an ephemeral loopback port for it and keeps the mapping, so the shards of a scaling run, which all listen on the same logical ports, never collide.
3. `abort` sets `SO_LINGER` to zero and closes the socket, as the kernel backend does, and dropping a session closes it with a FIN. `TCP_NODELAY` is on by default and `--nodelay off` turns it off. The kernel sizes the socket buffers unless `--buffer-kib` sets them.

What the numbers cannot show: kernel socket memory is not part of the resident set of the process, so `bytes_per_socket` is the user space cost of a socket only, and the stack has no frame counters. Every socket is a file descriptor, so a scaling run ends at the open files limit of the process, which the report lists as `open_files_soft_limit`. Reaching 1,000,000 sockets takes a limit above 1,000,100 and a high `fs.nr_open`.

### The own TCP stack

TRN-18 added `OwnStack`, the second measured candidate of decision record 0014. It is two TCP hosts, a client at 10.0.0.1 and a server at 10.0.0.2, that exchange IPv4 frames over a pair of `FrameIo` queues of `fps-backend-afxdp`. The default link is the in memory pair of that crate, so the stack needs no root, and TRN-12 can hand it AF_XDP queues through `OwnStack::with_links`. The only new dependency is the path crate `fps-backend-afxdp`, so `cargo deny check` passes with no new license. Sessions leave the stack as `SessionIo`, like those of the kernel stack.

TRN-21 moved the stack into its own crate, `engine/fps-tcp-own`, with `host.rs`, `wire.rs`, the clocks, and the component tests. `cargo tree -p fps-tcp-own` lists no smoltcp. The stack has the methods `listen`, `connect`, `accept`, `drive`, `pump`, `stats`, and `counters` of its own, so later packages (the datapath of TRN-27 and the host pools of TRN-32) use it without the harness. `fps-tcp` keeps the `TcpStack` adapter in `src/own.rs`, re-exports the types and the paths `fps_tcp::own::{host, wire}` and `fps_tcp::clock`, and still depends on smoltcp for the adapter of TRN-20. Behavior did not change: the run in `engine/fps-tcp/results/sandbox-2vcpu-trn21/` alternates the binary before and after the move and gives 33,500 to 35,500 sessions per second for both, and 1,000,629 sockets at 228 B with 56,000 probe sessions per second in the last scaling step for both.

TRN-32 gave the stack host pools. The connection table is keyed by the 4-tuple (both addresses and both ports, 12 bytes, 16 bytes per entry with the handle), so many local addresses and many peers share the same ports. Frames are Ethernet with an optional 802.1Q tag (`--own-vlan`), padded to 60 bytes, and host `i` of a pool has the MAC address `mac_base + i`. The neighbor of an address is read from the `NeighborTable` of `fps-hosts` (HST-03, a read only dependency), and the stack answers ARP for its hosts with the `Responder` of that crate. The stack has no IPv6, so neighbor discovery has no user. `OwnStack::with_pools` takes a `Pools` value, `connect` takes the client hosts in turn, and `connect_hosts` picks a pair. `tcp_bench` has `--own-pool N`, `--own-server-hosts N`, and `--own-vlan ID`. The run in `engine/fps-tcp/results/sandbox-2vcpu-trn32/` holds 1,001,129 sockets at 253 B each (228 B before: +8 B for the two addresses in a connection and +8 to 9 B for the wider table entry) with a pool of 20,000 client addresses. Its probe rate, 46,500 to 48,600 per second, equals the base binary in the same session (47,200 to 49,100), and the sandbox was about 14 percent slower than in the TRN-21 session, so the absolute acceptance of 50,000 per second needs a rerun on a quiet host.

| Part | What the stack does |
|---|---|
| Setup | SYN, SYN-ACK, and ACK with an MSS option. The initial sequence number is a counter that ticks every 4 microseconds plus a hash of the tuple and a secret (the idea of RFC 6528, with a mixing function and not a keyed cryptographic one). A SYN for a port without a listener gets a reset, which the client sees as `ConnectionRefused`. A new SYN may replace a TIME-WAIT tuple when its sequence number is higher |
| Data | Sequence and acknowledgment numbers, segments of at most MSS bytes, a receive window that follows the free space of the receive buffer, and a window update when a read opens the window by a worthwhile amount |
| Teardown | FIN in both directions with the states FIN-WAIT-1, FIN-WAIT-2, CLOSE-WAIT, CLOSING, LAST-ACK, and TIME-WAIT. Half close works: data that arrives after the own FIN is delivered. `abort` sends a reset, and the peer reads `ConnectionReset` after its data was discarded. Dropping a session closes it with a FIN, or with a reset when unread data is left |
| Loss recovery (TRN-23) | A round trip estimator and the retransmission timeout of RFC 6298 (minimum 200 ms, maximum 2 s, Karn's rule, backoff kept until a valid sample), fast retransmit on three duplicate acknowledgments with Limited Transmit, an out of order queue, and NewReno with an initial window of 10 segments. A one segment probe into a zero window, and a reset with `TimedOut` after 8 timeouts without any segment from the peer. See [Loss recovery in the own stack](#loss-recovery-in-the-own-stack-trn-23) |
| Options for long fat paths (TRN-24) | Window scaling and timestamps (RFC 7323, with PAWS), selective acknowledgment (RFC 2018 blocks, RFC 6675 recovery beside NewReno), and a delayed acknowledgment (every second segment or 40 ms, at once for out of order data). Each is offered in the SYN and used only when the peer offers it. See [Options for long fat paths in the own stack](#options-for-long-fat-paths-in-the-own-stack-trn-24) |
| CUBIC (TRN-25) | CUBIC of RFC 9438 is the default congestion control and NewReno stays selectable (`OwnConfig::congestion_control`, `tcp_bench --own-cc newreno\|cubic`). Both run on the same loss recovery. See [CUBIC in the own stack](#cubic-in-the-own-stack-trn-25) |
| OS profiles (TRN-28) | A side can take the TTL, MSS, SYN window, window scale, offered options, and option order of `windows11`, `linux6`, or `macos14` (`OwnConfig::client_os_profile` and `server_os_profile`), which the kernel backend can set only in part. See [OS profiles in the own stack](#os-profiles-in-the-own-stack-trn-28) |
| Checksums | The IPv4 header checksum and the TCP checksum are computed on send and verified on receive, so the numbers include them. A frame that fails a check is counted and dropped |

What it leaves out is the list that a production stack must still do, and decision record 0014 has to price it: no HyStart, no PRR, no simultaneous open, no urgent data, no IP fragmentation, no keepalive, no IPv6, and one peer address per host. TIME-WAIT lasts one second and not minutes, so a rate run does not run out of the 55,000 client ports.

Timers sit in a heap with at most one entry per connection, and a connection with no outstanding data has none. A poll with idle sockets therefore does not walk the sockets, which makes `idle_poll_us` flat in the socket count. A stack that walks its socket set, like smoltcp (see [The smoltcp adapter](#the-smoltcp-adapter)), grows with it. A connection is a slab entry of about 200 bytes with empty buffers, and the buffers exist only while data waits.

The tests in `engine/fps-tcp-own/tests/own.rs` run the stack through `SessionIo` over a link that records, drops, corrupts, and delays frames. They cover the handshake numbers, one initial sequence number per connection, bulk data in both directions, a small and a full window, half close, abort and refusal, a lost SYN, SYN-ACK, data segment, and FIN, a frame that arrives late, corrupted frames, a silent peer that times out, TIME-WAIT and its reuse after the port range wraps, a smaller MTU, and the three workloads.

### Running it

The command takes plain arguments and runs on any host without root.

```bash
cargo run --release -p fps-tcp --example tcp_bench -- --profile small
cargo run --release -p fps-tcp --example tcp_bench -- --stack own --profile small
cargo run --release -p fps-tcp --example tcp_bench -- --stack smoltcp --profile small
cargo run --release -p fps-tcp --example tcp_bench -- --stack own --profile full --label fat-nuc --out -
cargo run --release -p fps-tcp --example tcp_bench -- --workload scale --scale-steps 1000,100000 --shards 32
cargo run --release -p fps-tcp --example tcp_bench -- --help
```

The profile `small` finishes within a minute and fits a hosted runner. The profile `full` aims at 1,000,000 sockets over 64 shards. Every flag after the profile overrides it, and the stack flags `--nodelay`, `--buffer-kib`, and `--backlog` change the sockets. The command prints a report and, with `--out`, writes an `fps-bench/v1` document of the suite `tcp-stack` that `bench/compare.py` reads. The metric names start with the stack key, such as `kernel.rate.sessions_per_s`, `own.rate.sessions_per_s`, and `kernel.scale.10000.bytes_per_socket`. The flags `--own-mtu`, `--own-window-kib`, `--own-time-wait-ms`, and `--own-rto-ms` change the own stack, and the flags `--smol-mtu`, `--smol-checksums`, `--smol-buffer-kib`, `--smol-time-wait`, and `--smol-idle-poll-ms` change smoltcp. The fat-nuc run goes through the relay as a host test, see the handoff `docs/handoff/TRN-17.md`.

### First results of the kernel reference stack

The committed results in `engine/fps-tcp/results/sandbox-2vcpu/` come from the 2 vCPU agent sandbox with the default settings, which are `TCP_NODELAY` on and kernel sized buffers. The sandbox is not the GitHub hosted runner and not fat-nuc, it was shared with other workers, and its open files limit is 20,000, so TRN-10 compares only runs of one host and reads the scaling rows as a floor.

| Measure | Result |
|---|---|
| Sessions per second, 32 concurrent, 64 byte request and response | 14,000 to 35,000 between runs on the shared host, with a p50 of 0.8 ms, and no failures |
| Throughput, 4 concurrent uploads | about 20 Gbit/s on one core |
| Memory per socket (user space only) | 0.35 to 0.5 KB |
| Sockets reached | 10,177 and then 19,815, where the open files limit of 20,000 refused the next socket |
| Idle poll | zero, because the kernel has no user space poll |
| Probe session rate with 10,177 sockets held | about 28,000 per second |

The reference run says what the host gives a plain kernel session. It does not show the cost of one million sockets, which needs a host with a higher open files limit.

### Results of the own stack next to the kernel reference

The committed results in `engine/fps-tcp/results/sandbox-2vcpu-trn18/` come from the same 2 vCPU agent sandbox, run in one session, with the kernel reference run again next to the own stack, because the earlier kernel results of TRN-17 ran on an earlier boot with another clock. They are sandbox results, not results of the GitHub runner and not of fat-nuc, and the host was shared with other workers, so a rate varies by up to a factor of three between runs and differences below a factor of two are noise. The own stack runs over the in memory link and the kernel over loopback, and both hosts of a pair share one process and core.

| Measure | Own stack | Kernel reference |
|---|---|---|
| Sessions per second, 32 concurrent, 64 byte request and response | 32,000 to 36,000 in six runs, p50 0.8 to 0.9 ms | 14,000 to 49,000 in six runs, p50 0.63 to 0.69 ms |
| Sessions per second, 256 concurrent | 33,700 to 34,900, p50 6.3 to 7.1 ms | 29,200 to 32,800, p50 7.3 to 7.6 ms |
| Throughput, 4 concurrent 64 MiB uploads | 14.8 Gbit/s at MTU 1500, 15.9 Gbit/s at MTU 9000 | 19.0 Gbit/s in one run of this session, about 20 in TRN-17 |
| Sockets reached | 1,000,629 in 64 shards, in 0.76 s for the last step | 19,815 where the open files limit of 20,000 refused the next socket |
| Memory per socket (growth of the resident set) | 228 B at 1,000,629 sockets, 330 to 400 B below 10,000 | 0.31 to 0.50 KB, user space only, and kernel socket memory is not in it |
| Idle poll | flat at 0.4 us for one shard from 1,000 to 16,000 sockets, and 23 to 38 us for a pass over 64 shards up to 1,000,629 sockets | zero by construction |
| Probe session rate with the sockets held | 49,000 to 66,000 per second from 1,000 to 1,000,629 sockets | 14,000 to 43,000 per second from 1,000 to 16,000 sockets |

The own stack never retransmitted in these runs, because the in memory link loses nothing, so the numbers show the cost of the state machine, the checksums, the frame copies, and the adapter, and not the cost of loss recovery. The own stack counts the memory of its connection table, so its 228 to 400 B per socket is the whole cost of a socket, where the kernel's figure leaves out the kernel memory of a socket. It also holds 1,000,629 sockets with no descriptor per socket, which at 228 B per socket is about 230 MB of resident growth. A connection that waits for an accept or holds unread data costs more, by the size of its buffers. The rate of the own stack is limited by the latency of one driver pass and one task switch per step, so it hardly changes between 32 and 256 concurrent sessions, and the kernel is at the same level there.

### The smoltcp adapter

`SmolStack` (TRN-20) holds two smoltcp interfaces, a client host at 10.0.0.1 and a server host at 10.0.0.2, each with its own socket set, joined by an in memory IP link without delay or loss. A session is a `SessionIo` over one smoltcp TCP socket, and the future of `drive` polls both interfaces. smoltcp wakes the tasks that wait on a socket through the wakers of its `async` feature. The driver polls only after a session did something, and otherwise once per `--smol-idle-poll-ms` (100 ms), so the timers run while all sessions idle. Each listened port keeps a backlog of listening sockets, because a smoltcp listening socket serves one connection, and the adapter replaces every one that a SYN used. A dropped session closes its socket gracefully and a reap list removes it. A socket in TIME-WAIT is removed at once by default (`--smol-time-wait skip`), which models a stack that reuses a 4-tuple at once, and `keep` holds it for the ten seconds that smoltcp waits. The adapter is the draft of the first TRN-17 worker, preserved on `wp/TRN-20`, and TRN-20 merged it onto the harness that TRN-17 and TRN-18 shaped.

### Results of smoltcp next to the own stack and the kernel reference

The committed results in `engine/fps-tcp/results/sandbox-2vcpu-trn20/` come from the same 2 vCPU agent sandbox, with the three stacks run in one session, except the million socket step of the own stack and the 19,815 sockets of the kernel, which come from the runs of TRN-18 and TRN-17 on the same sandbox type. They are sandbox results, not results of the GitHub runner and not of fat-nuc. The fat-nuc run of smoltcp waits for the owner, because the relay account on fat-nuc has no Rust toolchain. The host test is in `docs/handoff/TRN-20.md`.

| Measure | smoltcp | Own stack | Kernel reference |
|---|---|---|---|
| Sessions per second, 32 concurrent, 64 byte request and response | 59,000 to 66,000 in three runs, p50 0.46 to 0.52 ms | 35,000 to 36,000, p50 0.83 to 0.86 ms | 42,000 to 45,000, p50 0.66 to 0.69 ms |
| Sessions per second, 256 concurrent | 50,000 to 53,000, p50 4.6 to 4.9 ms | 34,600 to 35,700, p50 6.6 to 7.0 ms | 14,000 to 33,000, p50 7.4 to 7.6 ms |
| Throughput, 4 concurrent 64 MiB uploads | 28.1 Gbit/s at MTU 1500, 30.4 at MTU 9000 | 13.4 Gbit/s at MTU 1500 | 22.5 Gbit/s |
| Sockets reached | 1,000,584 in 64 shards, in 12.6 s for the last step | 1,000,629 in 64 shards (TRN-18 run) | 19,815 in TRN-17, where the open files limit refused the next socket |
| Memory per socket (growth of the resident set) | 1.59 KB at 1,000,584 sockets, 1.6 to 1.9 KB below that, with 512 B receive and 512 B transmit buffers per socket | 228 B at 1,000,629 sockets (TRN-18 run), 350 to 410 B below 16,000 | 0.44 to 0.49 KB, user space only |
| Idle poll | 9 us at 1,016 sockets, 379 us at 16,016 sockets in one shard (24 ns per socket), 54 ms for one poll of 1,000,584 sockets in 64 shards (54 ns per socket) | 0.35 to 0.4 us for one shard, 23 to 38 us over 64 shards (TRN-18 run) | zero by construction |
| Probe session rate with the sockets held | 4,800 per second at 1,016 sockets, 870 at 4,016, 118 at 16,016 in one shard, and 60 at 1,000,584 | 33,000 to 68,000 per second up to 16,202 sockets | 29,000 to 43,000 per second up to 16,000 sockets |

smoltcp is the fastest stack of the three on the rate and the throughput workloads while few sockets exist. The rate comes with a caveat that a reader must keep: the pair uses TIME-WAIT skip, so a closed tuple is reused at once, and the kernel and the own stack hold a closed tuple longer. smoltcp walks every socket of a set on every poll and every frame, so the cost of one event grows with the sockets of that set. The idle poll per socket is 24 to 54 ns and the probe session rate falls by a factor of 40 between 1,000 and 16,000 sockets in one set. A user of smoltcp at a million sockets therefore needs many small sets with a flow table of its own in front of them, which is the layout of this run (64 shards of about 15,600 sockets, where the probe session rate is still only 60 per second). The memory per socket is 1.6 KB, of which 1 KB is the buffer pair that this run sets to 512 B each, so smoltcp is about 0.6 KB per socket without buffers, against 228 B for the own stack. The buffers are fixed per socket and cannot grow on demand, which a connection of the own stack does.

### The impaired link and the loss workload (TRN-22)

TRN-22 added a link with random loss, delay, reordering, and a rate limit with a bounded queue, and the workload `--workload loss` of `tcp_bench`, so every later child of TRN-12 measures against the kernel over the same link. The code is in `engine/fps-tcp`: `impair.rs` (the model, the retransmission counter, the `tc` commands), `lossy.rs` (the own stack), `smol.rs` (smoltcp, `SmolConfig::impair`), and `netns.rs` (the kernel).

- The model has three stages in the order of the kernel setup. A bottleneck serializes frames at the rate and drops a frame that finds more than the queue waiting (tail drop). Random loss removes a frame with the given probability. The delay holds the rest, and a share of frames skips the delay, so they overtake (`netem delay D reorder P`). Each direction has its own random sequence from a seed, so a run repeats its loss pattern. The settings are per direction, so 20 ms of delay is a round trip of 40 ms.
- The own stack lives in `fps-tcp-own`, which TRN-22 may not edit, so the impairment wraps its frame queues (`LossyOwn`, a `FrameIo` pair over the model), and a ticker pumps the stack when a held frame falls due. smoltcp takes the model in the pair driver.
- The kernel runs in a client, a router, and a server namespace. `tbf` (rate and queue) is the root of each router interface and `netem` (limit 100,000 packets, delay, loss, reorder) is its child. The impairment is in the router and not at the hosts, because the kernel sees a drop of its own qdisc as congestion at once, and `RetransSegs` then counts every retransmission that the drop refuses (7,900 to 14,200 per MiB in a first try). Offloads are off on the veth ends. Every transfer gets fresh namespaces, so the metrics cache of the kernel does not carry a slow start threshold over.
- Retransmissions of the own stack and smoltcp are data segments that start below the highest sequence number of their flow, counted where the sender hands the frame to the link. The kernel reports `RetransSegs` of its client namespace. A transfer is complete when the server has confirmed the byte count, and a transfer that does not complete within 60 s is reported with goodput 0.
- `sch_netem` is a kernel module, and the sandbox kernel does not have it. A hosted runner may not have it either (Ubuntu cloud kernels ship it in the extra modules package, which I could not check from here), and a package may not install packages, so the kernel rows at loss and delay need a host that has it (see the Shared page notes of the log entry of TRN-22).

Results from the agent sandbox (`engine/fps-tcp/results/sandbox-2vcpu-trn22/`), 3 transfers of 4 MiB per level, 20 ms in each direction (a round trip of 40 ms), not tuned:

| Loss | Own stack | smoltcp | Kernel |
|---|---|---|---|
| 0 % | 3 of 3 complete, 12.2 Mbit/s (2.75 s), no retransmission | 3 of 3, 10.3, 10.3, and 5.4 Mbit/s, about 40 resent segments without any loss | not measured, no sch_netem |
| 0.1 % | 3 of 3, 6.3 to 9.5 Mbit/s (mean 8.0), 88 to 142 retransmissions for 2 to 5 drops | 3 of 3, 8.2 to 12.3 Mbit/s (mean 11.0), 8 to 62 | not measured |
| 1 % | 2 of 3 (one timed out at 60 s), 2.3 and 3.1 Mbit/s, 727 to 1,008 retransmissions | 3 of 3, 3.9 to 8.1 Mbit/s (mean 5.4), 62 to 159 | not measured |

At 0 percent loss the own stack is limited by its 64 KiB window over a 40 ms round trip (65,535 B per 42 ms is 12.5 Mbit/s). At 1 percent a drop costs a retransmission timeout of 200 ms or more and a resend of the whole window, and one of three transfers did not finish in 60 s. This is the baseline of TRN-23. A matched link of 8 Mbit/s with a 64 KiB queue, no delay and no loss, which needs no netem, ran on all three stacks: the kernel reaches 7.69 Mbit/s with 347 to 349 retransmitted segments per MiB, the own stack 1.32 Mbit/s (706 retransmissions, 32 drops), and smoltcp 0.63 Mbit/s (371 retransmissions, 196 drops). The own stack and smoltcp recover from a tail drop queue far worse than the kernel, which is the baseline of TRN-23 and TRN-25.

### Loss recovery in the own stack (TRN-23)

TRN-23 gave the own stack the four mechanisms that the loss workload showed it lacked. The pure parts are in `engine/fps-tcp-own/src/recovery.rs` (the estimator, NewReno, the out of order queue), and `host.rs` wires them into the state machine.

| Mechanism | What the stack does |
|---|---|
| Round trip estimator | RFC 6298: the first sample sets `SRTT = R` and `RTTVAR = R/2`, later ones use gains of 1/8 and 1/4, and the timeout is `SRTT + max(G, 4 RTTVAR)` with `G` of 1 ms, kept between `min_rto_us` (200 ms, as the kernel does, where the RFC says 1 s) and `max_rto_us` (2 s). One segment is timed at a time (Karn's rule), a segment that is sent again is not timed, and a timeout doubles on every expiry and keeps its backoff until an acknowledgment of a segment that was sent once arrives. The timeout goes back to the initial value when the handshake completes |
| Fast retransmit | The third duplicate acknowledgment (same acknowledgment number, no data, no SYN, FIN, or RST, the same window, data outstanding) sends the oldest segment again without waiting for the timer. The first two send new data if the windows allow (Limited Transmit, RFC 3042). After a timeout, duplicates that do not reach `recover` start no new recovery (RFC 6582) |
| Out of order queue | A segment inside the receive window behind a hole is held (the bytes held never exceed the window), the receiver sends an ACK at once for each one so that the sender counts duplicates, and when the hole fills the held bytes join the stream. A FIN behind a hole is remembered and taken when the hole fills, so the sender does not send it again |
| NewReno | Slow start grows the window by the bytes acknowledged, up to the threshold, and congestion avoidance by about one segment per round trip. Fast recovery follows RFC 6582: the threshold falls to half the flight, the window inflates by a segment per duplicate, a partial acknowledgment sends the next hole and deflates, and a full one sets the window to the threshold. A timeout sets the window to one segment and goes back N from the first unacknowledged byte. The initial window is 10 segments (RFC 6928, `initial_cwnd_segments`) |

Two choices differ from the text of the RFCs. Slow start counts all acknowledged bytes and not one segment per ACK (RFC 5681 equation 2), because the stack acknowledges a whole batch of segments with one ACK, and a limit of one segment per ACK made the window grow far slower than a doubling per round trip. And the receiver sends a window update when a read opened the window by a segment or half the buffer since the last window it sent (RFC 1122 section 4.2.3.3); the earlier rule fired only when the window had been nearly closed, which halved the goodput at 0 percent once the congestion window kept the flight below the receive window.

**Memory.** The state of these mechanisms sits in `Option<Box<Recovery>>` of the connection. A connection creates the box when it sends data or holds data behind a hole, and drops it when nothing is in flight, nothing waits to be sent, and nothing is held. An idle connection holds a `None` of 8 bytes, so the cost per socket stays flat. At 1,000,000 sockets the own stack takes 261 and 262 bytes per socket after TRN-23 against 253 and 255 on main in the same session (+8), with a probe rate within 4 percent of main. The price of dropping the box is that an idle connection restarts slow start from the initial window (which RFC 5681 section 4.1 asks for after an idle time anyway) and forgets `SRTT` and the threshold; its retransmission timeout stays.

**Results.** `engine/fps-tcp/results/sandbox-2vcpu-trn23/` has the loss run, the 8 Mbit/s run, and the scale rows with main, in one session of the 2 vCPU sandbox. At 20 ms per direction:

| Loss | Own before TRN-23 | Own after TRN-23 |
|---|---|---|
| 0 % | 12.2 Mbit/s | 12.0 Mbit/s, 3 of 3 |
| 0.1 % | 8.0 Mbit/s, 111 retransmissions | 7.5 Mbit/s, 3 of 3, 2 to 5 retransmissions |
| 1 % | 1.8 Mbit/s, one transfer of three unfinished | 3.1 Mbit/s, 3 of 3 (2.7, 3.0, and 3.7), 15 to 34 retransmissions |

The hosted runner measured the kernel at 4.2 Mbit/s at 1 percent (PR #642), so the acceptance threshold is 2.1 Mbit/s. The sandbox gives 3.1 for the own stack, and the kernel row of the same link needs `sch_netem`, which the sandbox lacks, so the comparison is made by the coordinator on the hosted runner. At 0.1 percent the own stack is a little below the stack before it, which resent a whole window per drop and was limited only by the 64 KiB window. At 8 Mbit/s with a 64 KiB queue it goes from 1.3 to 7.7 Mbit/s, as the kernel does, with 1 retransmission per transfer in place of 706.

Tests: the estimator, Karn's rule, the duplicate rule, NewReno transitions, and the queue have unit tests in `recovery.rs`. `host/recovery_tests.rs` runs two hosts by hand with a clock that the test sets and drops chosen segments: a fast retransmit without the timer, a partial acknowledgment, a lost tail, a FIN behind a hole, Karn with exact timeouts, and the state existing only in flight. `tests/lossy.rs` has the 1 percent transfer and a reordering transfer.

### Options for long fat paths in the own stack (TRN-24)

TRN-24 gave the own stack the four options that a path with a bandwidth delay product above 64 KB needs. The wire format is in `engine/fps-tcp-own/src/wire.rs` (`Options`, encoding and parsing), the sender side of selective acknowledgment in `engine/fps-tcp-own/src/sack.rs` (the scoreboard), the receiver blocks and the SACK entry into NewReno in `recovery.rs`, and `host.rs` wires them into the state machine. Each option is a switch of `OwnConfig` (`window_scale`, `timestamps`, `sack`, `delayed_ack`, and `delayed_ack_us`), and `tcp_bench` has `--own-wscale`, `--own-timestamps`, `--own-sack`, and `--own-delayed-ack` with `on` or `off`.

| Option | What the stack does |
|---|---|
| Negotiation | The SYN offers every enabled option, in the order of the kernel (MSS, SACK permitted with the timestamp, window scale). The SYN-ACK answers only the options that the SYN carried, and the client keeps only the options that the SYN-ACK carries. A peer without an option gets a plain 20 byte header for it: the stack falls back option by option, and a peer that offers nothing gets the stack of TRN-23. The window of a SYN is never scaled |
| Window scaling (RFC 7323) | The shift is the smallest one that brings the receive buffer under 65,536 (5 for the default of 1 MiB, at most 14). The window field is the free space shifted right, and the window of the peer is shifted left by the shift in its SYN. The default buffers grew from 65,535 and 128 KiB to 1 MiB each, and `--own-window-kib` takes up to 1 GiB where it clamped to 65,535 |
| Timestamps (RFC 7323) | The value is the millisecond clock plus an offset per host. The ACK echoes `TS.Recent`, which follows the segments that start at or before the next byte expected and keeps the older value while an ACK is delayed, so the sender's sample includes the delay. An ACK that moves `snd_una` gives a round trip sample from the echo (1 ms resolution, valid for a segment that was sent again too), where the timed segment of Karn's rule, which has the finer clock, wins when there is one. PAWS drops a segment with an older value (signed 32 bit comparison, so the wrap works) and answers with an ACK, and a segment without the option on a connection that agreed on it is dropped as RFC 7323 section 3.2 asks. A reset is not checked. The 12 bytes of the option come off the payload, so the MSS of the connection is 1,448 and the frame still fits the MTU |
| SACK, receiver (RFC 2018) | Every ACK sent while data waits behind a hole carries blocks from the out of order queue: the run that holds the segment that arrived last first, then the others from the lowest sequence number, up to four blocks (three beside a timestamp) |
| SACK, sender (RFC 6675) | The scoreboard holds the SACKed ranges above `snd_una` and the ranges sent again. A hole is lost when three separate ranges or more than two segments of bytes lie above it. The third duplicate acknowledgment, or the first one that makes the first hole lost, starts a recovery with `ssthresh = flight / 2` and `cwnd = ssthresh`, without NewReno's inflation. While `cwnd - pipe` leaves room for a segment the sender sends the lowest lost range that was not sent again, then new data; a partial acknowledgment sends nothing by itself. A timeout clears the scoreboard and goes back N as before. A peer without SACK gets the NewReno of TRN-23 |
| Delayed ACK (RFC 1122, RFC 5681) | The first in order segment waits for a second one or for 40 ms (`delayed_ack_us`). Everything else is answered at once: data behind or at a hole, a duplicate, data that did not fit, a FIN, a window update after a read, and any segment while a timeout is armed. A response carries the ACK and the timer finds nothing to do |

Choices that differ from the text of the RFCs, each with its reason:

- The hole at `snd_una` counts as lost during a SACK recovery whatever lies above it. The acknowledgment that moved `snd_una` to it was a partial one, which NewReno reads as a lost segment, and a tail with fewer than three segments above its last hole would otherwise wait for the timeout.
- A timeout restarts when the oldest segment is sent again, as the kernel does. Without it a recovery that lasts a round trip ran into the timer that the last advancing ACK had set, on a path whose round trip grows with its queue (the test `a_retransmission_of_the_oldest_segment_starts_the_timeout_again`).
- The delayed ACK uses the one timer of the connection and so is not delayed while a timeout is armed, which a connection that sends and receives at once would meet. The ACK then goes out at once, which costs acknowledgments and never time.
- The 24 day rule of PAWS (a `TS.Recent` that is older than 24 days is not trusted) is not implemented, and neither is D-SACK (RFC 2883): a block below `snd_una` or inside a held range is ignored. The receiver does not send D-SACK blocks.
- The scoreboard does not detect a lost retransmission. That waits for the timeout, as in RFC 6675.

**Memory.** The option state of a connection is 6 bytes (`opts: u16` with the three flags, both shifts, and the flags of the delayed ACK, and `ts_recent: u32`). The timeout became a `u32` of microseconds and one flag moved into the option bits, so `Conn` is 176 bytes as before (a test keeps it there). The scoreboard (two small vectors) and the SACK blocks are made inside the `Recovery` box, so an idle connection holds none. At 1,000,000 sockets: 262 bytes per socket with the options (262 and 263 in two runs), against 262 and 262 for main in the same session, alternating main, branch, main, branch. The idle poll stays at 28 to 29 us.

**Results.** `engine/fps-tcp/results/sandbox-2vcpu-trn24/` has the loss rows and the scale rows in one session of the 2 vCPU sandbox. The acceptance row is `tcp_bench --stack own --workload loss --loss-profile fat`: 0.1 percent loss, 50 ms of delay **in each direction** (a round trip of 100 ms, as in TRN-22 and TRN-23, where the delay is per direction), 50 Mbit/s with a queue of 1 MiB, so the bandwidth delay product is 625 KB, and 8 MiB per transfer, six transfers with fixed seeds. Mean goodput over the transfers:

| Row | Goodput, mean of 6 | Completed |
|---|---|---|
| Own after TRN-24 (all options) | 13.40 Mbit/s (4.6 to 26.3 per transfer) | 6 of 6 |
| Own on main (TRN-23) | 3.51 Mbit/s | 6 of 6 |
| Own after TRN-24, only window scaling on | 14.07 Mbit/s | 6 of 6 |
| smoltcp | 4.92 Mbit/s (window capped at 64 KiB) | 6 of 6 |
| Kernel | not measured in the sandbox (no `sch_netem`); the coordinator reads it from the tcp-bench workflow, job `loss` | - |

The default loss run (`--workload loss`, levels 0, 0.1 and 1 percent, 20 ms each way, no rate) still works: 52.8 Mbit/s at 0 percent, 13.8 at 0.1 percent, 3.2 at 1 percent, 3 of 3 transfers complete at each level.

The own stack gains about 3.8 times over TRN-23 on this path, and 2.7 times over smoltcp. The 80 percent of the kernel is **not verified**: the sandbox cannot run the kernel row, so the ratio waits for the hosted runner. The window scaling accounts for the whole gain on this seeded link: with SACK, timestamps and delayed ACK off the mean is 14.07 against 13.40 with them on, within the spread of the transfers (a transfer takes 2 to 15 seconds depending on where the few drops (2 to 17 per transfer) fall). SACK repairs several holes in one round trip in the unit tests, but a link that drops one segment in a thousand seldom drops two in one window. A run with loss above 0.1 percent would show the difference.

The hosted runner measured the kernel at 24.09 Mbit/s on this row against 13.17 for the own stack (55 percent). The kernel runs CUBIC with HyStart by default and the own stack runs NewReno (CUBIC is TRN-25), so the row compares two congestion controls as much as two stacks. `tcp_bench --kernel-cc NAME` sets `TCP_CONGESTION` on the client and server sockets of the kernel row (default: the system choice), so `--kernel-cc reno` gives the like for like row. The own stack has no cheap win left on this path for TRN-24: without loss it moves 8 MiB at 34 Mbit/s in 2.0 s, which is what slow start from 10 segments to the 625 KB product costs (about 0.55 s of ramp), the queue never overflows, there are no timeouts, and the windows are large enough. Under 0.1 percent loss a transfer sees 4 to 5 SACK recoveries, each halving a window that NewReno regrows by one segment per round trip (about 20 s from half the product), so the rate follows the Reno bound of about 4.5 Mbit/s at that loss and round trip, lifted by the short transfer that sees few losses.

Tests: `wire.rs` (the layout of a SYN with every option against the kernel's, the padding, three or four blocks, the cap of the shift, malformed and unknown options), `sack.rs` (the scoreboard: merge, trim, `IsLost`, `pipe`, `NextSeg`, the wrap), `recovery.rs` (the blocks and their order, NewReno with SACK), and `host/recovery_tests.rs` (the SYN and SYN-ACK, negotiation of all combinations and the fallback of each option, the window field, a scaled window, timestamps echoed and sampled where Karn's rule gives none, PAWS with the wrap and a reset, blocks in the ACK, several holes in one round trip against one per round trip, a hole with little above it, a lost retransmission, a long transfer with a loss every 17th segment, the delayed ACK timing in six cases, the restart of the timeout). `tests/lossy.rs` moves 2 MiB over 40 Mbit/s with 25 ms each way and needs the scaled window to pass 15 Mbit/s.

### CUBIC in the own stack (TRN-25)

TRN-25 added CUBIC (RFC 9438) beside the NewReno of TRN-23. `engine/fps-tcp-own/src/cubic.rs` holds the window function and its state, `recovery.rs` calls it from `NewReno` (the struct keeps the loss recovery: duplicate acknowledgments, `recover`, SACK entry, partial acknowledgments, timeout), and `host.rs` passes the time, the smoothed round trip time, and the flight with every acknowledgment. The choice is `OwnConfig::congestion_control` (`CongestionControl::Cubic`, the default, or `NewReno`), and `tcp_bench --own-cc newreno|cubic` sets it. CUBIC is the default because it is the default of the Linux kernel that the own stack is measured against.

| Rule | CUBIC in the own stack |
|---|---|
| Slow start | The rule of RFC 5681, as in NewReno. There is no HyStart: slow start leaves on a loss |
| Event (fast retransmit, SACK recovery) | The threshold is 0.7 of the flight and `W_max` the window, lowered to 0.85 of the window when it fell short of the last `W_max` (fast convergence, RFC 9438 section 4.7) |
| Congestion avoidance | `W_cubic(t) = C (t - K)^3 + W_max` with `C` = 0.4 and `K` the cube root of `(W_max - cwnd) / C`; the target is the curve one smoothed round trip ahead, at most 1.5 times the window; the reno friendly estimate (alpha 0.53, then 1) is the floor |
| Timeout | The threshold is 0.7 of the flight and the window one segment; a timeout inside a recovery does not lower `W_max` a second time |
| Idle time | The epoch moves by the time the flight was zero (RFC 9438 section 4.2) |

The code is a port of parts of smoltcp 0.14.0 (0BSD), file `src/socket/tcp/congestion/cubic.rs`: the constants, `cube_root`, the window function, the reno friendly estimate with its `alpha` switch, the 1.5 clamp, the idle absorption, and the event rules. The header comment of `cubic.rs` lists them and the differences: the epoch starts at the first acknowledgment after an event and `K` uses the window at that moment, an acknowledgment counts all its bytes (a batch is one ACK here), and the fraction of a byte that a step does not add carries over. The state is 72 bytes in the `Recovery` box, so an idle socket pays nothing: the scale row of the full profile reads 263 bytes per socket at 1,001,129 sockets (262 and 263 before).

`tcp_bench` has a fairness variant of the loss workload. `--loss-flows N` makes the run one of N flows over the one link: every flow sends without end, the server counts the bytes of each flow, and the result is the bytes in a window (`--loss-window-secs`, default 10) that opens `--loss-warmup-secs` (default 5) after the last flow connected. The goodput is the sum over the window and the Jain index is `(sum x)^2 / (n sum x^2)` over the flows. `--loss-profile bottleneck` sets 4 flows, 0 percent loss, 20 ms in each direction, 1000 Mbit/s with a 4 MiB queue, and for the own stack 4 MiB windows and send buffers (`--own-sndbuf-kib` is new, because `--own-window-kib` sets the receive side only). The kernel runs the flows as one client process and one server process in the namespaces of the loss workload.

Sandbox results (2 vCPU, release build; `engine/fps-tcp/results/sandbox-2vcpu-trn25/`, no kernel row because the sandbox has no `sch_netem`):

| Row | Own CUBIC | Own NewReno | smoltcp |
|---|---|---|---|
| Fat path, 0.1 percent loss, mean of 6 | 20.00 Mbit/s | 14.18 | 4.91 (64 KiB window) |
| Fat path, mean of 24 (seed 7) | 19.58 Mbit/s | 13.54 | not run |
| Four flows, 1 Gbit/s, 20 ms: goodput (mean of 6) | 891 Mbit/s | 891 | 51 (64 KiB windows) |
| Four flows: Jain index, mean (lowest run) | 0.941 (0.776) | 0.962 (0.932) | 1.000 |

The fat path row has a large spread (6 to 32 Mbit/s per transfer) because a loss early in slow start leaves a small window that CUBIC (like the kernel) regrows slowly, so the mean of six moves by a few Mbit/s between runs. The acceptance reference (kernel 24.09, so 19.3 needed) is met with a thin margin of 1 to 4 percent in the sandbox; the coordinator reads the kernel rows on the hosted runner. Four flows fill the link at 891 Mbit/s of payload (the rate counts the frame bytes), and the Jain index is at or above 0.93 in 11 of the 12 runs of the two controls; one CUBIC run read 0.776 (two flows at 92 and 118 Mbit/s against 313 and 368), so a single window of 10 s can miss 0.9 although the mean of six does not.

Tests: `cubic.rs` (cube root, `W_max` and `K`, fast convergence, the floor of two segments, the 1.5 clamp, the reno friendly region, alpha, idle time, the timeout rules, the recovery exit), `recovery.rs` (CUBIC with fast retransmit, SACK recovery, the recover guard, timeout, fast convergence across events, growth against NewReno at a large window), `host/recovery_tests.rs` (the default, the 0.7 reduction, a timeout, a SACK recovery that repairs four holes, a lost retransmission that falls back to the timeout), `workload.rs` and `tests/lossy.rs` (the Jain index and a two flow run over a rate limited link), `cli.rs` (`--own-cc`, the bottleneck profile and its overrides), `report.rs` (the fairness metrics), and `netns.rs` (the server result line of a fairness run).

Run for the acceptance rows (the loss job of the `tcp-bench` workflow takes these as `loss_args`):

- Fat path, one flow: `--loss-profile fat --loss-transfers 6` for kernel, own, and smoltcp, and `--loss-profile fat --loss-transfers 6 --own-cc newreno` for the NewReno row.
- Four flows: `--loss-profile bottleneck --loss-transfers 3` for kernel, own, and smoltcp.

### OS profiles in the own stack (TRN-28)

TRN-28 maps the [operating system profiles](#operating-system-profiles) onto the SYN and the SYN-ACK of the own stack. The kernel backend reaches them through socket options and sysctls and cannot write the option order of Windows 11 and macOS 14, because Linux writes a fixed order. The own stack builds its frames, so it reproduces every row of the table in [Profile files](#profile-files), the option order included.

`OwnConfig::client_os_profile` and `server_os_profile` take an `OsProfile` (`engine/fps-tcp-own/src/profile.rs`; `OsProfile::builtin("windows11")` and the constants `WINDOWS11`, `LINUX6`, `MACOS14`). A test configuration names a profile per host pool, so each side has its own. A side without a profile is unchanged (TTL 64, the option order of the kernel, a window scale from the buffer).

| Field of the table | In the own stack |
|---|---|
| `socket.ttl` | The IP TTL of every frame of the side, which `Style` in `wire.rs` writes. A reset carries it too |
| `socket.mss` | The MSS option of the SYN and the SYN-ACK, lowered to the link MTU less 40 bytes, as `TCP_MAXSEG` only lowers |
| `reference.syn_window` | The window field of the SYN and the SYN-ACK, which is never scaled, lowered to the receive buffer |
| `reference.window_scale` | The shift of the option. The window field after the handshake is the free space shifted right by it |
| `system.window_scaling`, `system.timestamps`, `system.sack` | Whether the side offers each option, in place of the `window_scale`, `timestamps`, and `sack` fields of `OwnConfig`. A SYN-ACK answers only what the SYN offered, as before |
| `reference.options` | The order of the options of the SYN. The SYN-ACK has the same order, and an option that is not in it (the peer did not offer it, or the side does not) leaves with the NOPs that belong to it. Zero bytes fill the options to a multiple of 4, so the `eol` of macOS 14 is followed by one more zero |
| `reference.dont_fragment` | Always on, as for every frame of the stack |
| `socket.window_clamp`, `system.tcp_rmem_max` | Not used. They only produce the window scale in the kernel, and the own stack takes the scale from `reference.window_scale` |

Linux 6 keeps the writer that the stack had before: its order is the order of the kernel, and a SYN-ACK without the timestamp pads as the kernel does (`nop nop sok`, then `nop ws`).

A Windows 11 side offers no timestamp whatever `OwnConfig::timestamps` says, so a connection between a Windows 11 client and a Linux 6 server runs without timestamps and with the window scales 8 and 7.

Tests (no root):

- `engine/fps-tcp-own/tests/profile_table.rs` reads the table of this page and compares every value of the three profiles: TTL, MSS, SACK, timestamps, window scaling, window scale, SYN window, and the option list. A change of the table or of the profile fails it.
- `src/host/profile_tests.rs` writes the expected bytes of the table by hand and compares the SYN and the SYN-ACK of each profile with them: the TTL, the do not fragment flag, the window, and the option bytes with the padding. It also covers the SYN-ACK of each profile against clients with other options, the TTL of the ACK and of a reset, the profile winning over the option fields, the MSS at an MTU of 9000, 1500, and 1000, and the negotiated shifts.
- `tests/own.rs` runs each profile on both sides through `SessionIo` with a transfer in both directions, and reads the TTL, window, and option names of the SYN and the SYN-ACK and the TTL of every frame off the recorded frames.
- `src/wire.rs` tests the ordered writer: the kernel order gives the bytes of the built in writer, padding follows the option that it belongs to, and an overlong order drops what does not fit.

The example `os_profile_capture OUTDIR [PROFILE...]` records one short session per profile into `OUTDIR/PROFILE.pcapng`, so the capture checks of the kernel backend run on the own stack without root or a namespace:

```bash
cargo run -p fps-tcp-own --example os_profile_capture -- "$OUT"
python3 -m realism score --pcap "$OUT/windows11.pcapng" --labels LABELS --parsers none \
  --os-profile windows11 --client-hops 0 --server-hops 0 --transport-strict --output "$OUT/windows11.json"
```

The capture is one link, so no router lowers the TTL: the hops are 0 where the kernel test uses 1 on the server side. The runner exits 1 because its conformance check fails on a protocol that nDPI does not know, as in the kernel test, and the verdict is the `transport_behavior` entry of the report. In strict mode, which also compares the option order and the SYN window, the entry passed with no failure and no deviation for all three profiles, and each capture scored as another profile (windows11 as linux6, linux6 as macos14, macos14 as windows11) failed it on the TTL, the window scale, and the option list. The kernel test runs windows11 and macos14 non strict, because the kernel cannot write their order.

Limits:

- A profile is the SYN. The behavior after the handshake (the initial window of 10 segments, the retransmission timing, the initial sequence numbers, ECN) is the one of the own stack for every profile, as for the kernel backend.
- The SYN-ACK of a real macOS 14 or Windows 11 may differ from its SYN in ways that public signatures do not record. The stack sends the SYN order for both.
- Only IPv4 (the stack has no IPv6), and the three built in profiles. A profile of another system is a value of `OsProfile` with its option list, and the YAML profiles of the kernel backend are not read by this crate.

### The user space datapath (TRN-27)

`engine/fps-datapath` runs the own stack on the queues of a port. It is a crate of its own because `fps-hosts` and the stacks depend on `fps-backend-afxdp` for `FrameIo`. It does not set `TRANSPORT_AVAILABLE` and does not change the backend selection: AGT-24 makes a forced AF_XDP run work, and TRN-31 flips the flag.

A `Lane` is the unit of one thread: the queue of the client arm, the queue of the server arm, and the host pools (a slice of the test configuration) that the two arms serve. The own stack keeps one host per arm and owns both links, so a lane has exactly the two queues that the stack wants. `split_pools(pools, n)` cuts the pools into one slice per lane. A client host reaches only the server hosts of its own lane, and the receive side steering of a real port must send a flow to the queue of its lane (that is TRN-19 and the XDP program, not this crate).

```text
wire -> queue -> Splitter -+-> IPv4 and IPv6 TCP -> OwnStack (hosts, sessions, timers)
                           +-> ARP, NS -------------> fps-hosts Responder (reply sent by the splitter)
                           +-> IPv4 and IPv6 UDP --> UdpArm of fps-udp-own (TRN-29, TRN-30)
                           +-> other ND, DHCP ------> FrameHook (answers Unsupported for now)
wire <- queue <------------------ frames of the stack and the replies of the splitter
```

The `Splitter` wraps the `FrameIo` of one queue and is a `FrameIo` itself, so `fps-tcp-own` needed no change: the stack sees IPv4 TCP frames only. `classify` reads the Ethernet header with its 802.1Q tags, the IPv4 header up to the protocol and for UDP the two ports, and the IPv6 header up to an ICMPv6 neighbor discovery type, and it never panics on a short frame. A frame with another VLAN tag than the port has is counted in `vlan_dropped`. ARP requests for a host of the lane are answered by the `Responder` of `fps-hosts` from the same MAC addresses that the stack puts in its frames (`Pools::tables` is the one place that claims them, and the stations of the stack and the responders of the splitters take their tables from it, since TRN-33). The replies leave on the queue that took the request. ND, DHCP, and UDP frames go to a `FrameHook`; the default hook answers `Unsupported` and the splitter counts it (`nd_unsupported`, `dhcp_unsupported`, `udp_unsupported`). `DatapathConfig::hooks` builds the hooks per lane and side, which is where TRN-29 plugs UDP in. TRN-37 plugs `DhcpHosts` in for the arms whose pool leases (see below), so `dhcp_unsupported` stays 0 there. Since TRN-30 the splitter answers IPv6 neighbor solicitations itself, as it does ARP, and only the other ND messages reach the ND hook. Everything else is counted in `other_dropped`.

One thread per lane polls `OwnStack::pump`. `OwnStack::drive` does not fit a queue that the kernel fills, because it sleeps until the next timer or a notification and nothing notifies it when a frame arrives from the wire. A thread that found nothing spins for `spin_passes` (256) passes and then sleeps `idle_sleep` (30 microseconds), so an idle lane costs little CPU and a busy lane never sleeps.

The calls mirror `fps-backend-kernel`:

| Kernel backend | Datapath |
|---|---|
| `Connector::connect(source, destination)` | `Datapath::connector()` gives a `Connector` that is `Clone`; `connect(source, destination)` opens a session from a client host address to a server host address of the same lane and fails with `UnknownSource`, `UnknownDestination`, `ConnectTimeout`, or `Stack` |
| `KernelListener::bind(address, config)` and `accept` | `Datapath::listen(port)` listens at every server host of every lane, and `Listener::accept` returns the next session from any lane with the address of its peer, as `KernelListener::accept` does since TRN-33. Dropping the `Listener` frees the port |
| `KernelStream` | `DatapathSession`, the `OwnSession`, which is `SessionIo`, `AsyncRead`, and `AsyncWrite`, with `local_addr` and `peer_addr` that return `io::Result<SocketAddr>` as the kernel stream does (TRN-33) |
| IPv6 address | `connect` and `connect_udp` take the family from the addresses since TRN-30; the pools decide which family a lane has (see the IPv6 section below) |
| `Connector::connect_datagram` and `KernelDatagramListener` | `Connector::connect_udp` and `Datapath::listen_udp` since TRN-29, see the section below |

`Monitor` (`Datapath::monitor()`) reads `sockets_open` and the counters of every lane (`LaneStats`: the stack, the `FrameCounters` of both splitters, and the loop passes) from any task, and `Datapath::stop()` joins the threads and returns a `LaneReport` per lane.

#### Tests

- `engine/fps-datapath/tests/memory.rs` runs in every job and needs no root. Two lanes over in memory pairs carry 40 sessions with a 300 byte request and a 3,000 byte answer and check every byte; a tagged pool carries a session and answers tagged ARP only; the ARP reply carries the MAC address of the neighbor table; ND, DHCP, UDP, other protocols, an unknown Ethernet type, and a short frame are counted as unsupported, dropped, or malformed; a hook takes frames and its reply leaves on the same queue; the unsupported calls and the address errors return their errors; the pools split into lanes that cover each host once.
- `engine/fps-datapath/tests/listener.rs` (TRN-33, no root) checks that `accept` returns the peer and that both ends of a session know their addresses, that a port can be listened on again after its `Listener` is dropped (three rounds on two lanes, the accepted session outliving its listener and a new connect refused), and that a second `listen` on a port in use fails without taking the port from the first listener.
- `engine/fps-datapath/src/classify.rs` has unit tests of the split, the tags, short frames, and every prefix of a frame.
- `engine/fps-backend-afxdp/tests/datapath_veth.rs` is the acceptance test. It makes a namespace with a veth pair, `dpa` and `dpb`, runs itself again inside it, opens an AF_XDP queue on each end, and gives them to one lane as the client arm and the server arm. The hosts are two pools of 64 hosts. The helper holds a number of sessions open (connect, 64 byte request, 1,024 byte response, no close), reads `sockets_held` (both ends of every held session count), and then runs short sessions (connect, request, response, close) from 64 concurrent tasks while it still holds the first ones. It checks every response byte. The test lives in `fps-backend-afxdp`, which has `fps-datapath` as a dev-dependency (Cargo allows a cycle through dev-dependencies), so the `afxdp` job of `trn.yml` runs it with `FPS_REQUIRE_AFXDP=1` and no job of its own. It needs root (`sudo -n` when it is not root), skips with a message where that is impossible, and fails there when `CI=true` or `FPS_REQUIRE_AFXDP=1`. `FPS_DP_SESSIONS` (10,000), `FPS_DP_HOLD` (2,000), `FPS_DP_CONCURRENCY` (64), and `FPS_DATAPATH_RESULTS` (a directory that receives `datapath-veth.txt` and `host.txt`) set the run.

```bash
cargo test -p fps-datapath                                   # no root
FPS_DATAPATH_RESULTS=/tmp/dp FPS_REQUIRE_AFXDP=1 cargo test -p fps-backend-afxdp --test datapath_veth -- --nocapture
```

#### Results

| Run | Sessions per second | Sockets held | Sessions | Held | Concurrency |
|---|---|---|---|---|---|
| Hosted runner, `afxdp` job, debug build (`results/ci-ubuntu-24.04-trn27/`) | 6,908 | 4,001 | 10,000 | 2,000 | 64 |
| Sandbox, 2 vCPU, debug build, two runs (`results/sandbox-2vcpu-trn27/`) | 3,181 and 3,122 | 4,001 | 10,000 | 2,000 | 64 |
| Sandbox, 2 vCPU, release build, three runs of 50,000 sessions (`results/sandbox-2vcpu-trn27-release/`, the last run) | 16,453, 17,046, and 16,070 | 4,001 | 50,000 | 2,000 | 64 |

Every run completed all its sessions with no failure and no wrong byte, and both queues bound in copy mode (a veth pair has no zero copy driver). `sockets_held` is the sockets that the stack holds with 2,000 sessions open: 4,000 for the two ends and one for the listener. The peak during the churn is higher (11,176 on the runner, 21,000 to 22,600 in the release runs) because closed sessions stay in TIME-WAIT. The splitters counted the neighbor discovery frames that the kernel sends when a veth pair comes up (3 in `nd_unsupported` and 4 in `other_dropped` on the runner), which shows the unsupported path on a real port. The rate is the whole path: the own stack, two AF_XDP sockets in copy mode, the veth driver, and a polling thread that sleeps 30 microseconds when idle, in one namespace on two to four cores that the helper, the test tasks, and the kernel share. It is the first number for tier T1 on real queues and not a limit of the design: the debug build is 2.2 to 5 times slower than release, and a host with native XDP and zero copy (TRN-19) is the measurement that decides.

#### Limits

| Limit | Consequence |
|---|---|
| A lane has one client arm and one server arm | The stack models both ends of a test, so the agent gives it the two ports (or queues of two ports) of the device under test. A run with only a client arm needs a stack with one side |
| A client host reaches only the server hosts of its lane | `split_pools` cuts both pools evenly. A mixed pair across lanes needs the steering of TRN-19 and a stack that takes frames for a host of another lane |
| Polling threads | A lane thread polls and sleeps 30 microseconds when idle, which adds up to that much to a round trip on an idle lane. Busy polling or `wait_readable` on both queues is a TRN-19 question |
| No CPU pinning | `WorkerOptions::cpus` of the AF_XDP crate pins workers with `sched_setaffinity`, which this crate (no `unsafe`) does not call yet |
| DHCP, and ND that is not a solicitation for a host | Counted and answered with `Unsupported`, see the hooks above. UDP is served since TRN-29, IPv6 and neighbor solicitations since TRN-30 |

### UDP over FrameIo (TRN-29)

`engine/fps-udp-own` is a user space UDP layer behind `fps_io::DatagramIo` over Ethernet and IPv4 frames, so the UDP modules (dns, ntp, sip_rtp) run on the AF_XDP datapath the way they run on the kernel backend. It has no thread and no kernel socket, and it depends on `fps-io` and `fps-tcp-own` (for `Station`, `Pools`, and the Ethernet framing) only. `fps-datapath` puts one `UdpArm` behind the UDP hook of each arm of each lane.

```text
task --poll_send--> UdpSession --frames--> outbox --drain_tx (FrameHook::poll_tx)--> splitter --> queue
task <--poll_recv-- UdpSession <-datagram- UdpArm::receive (UDP hook) <-- splitter <-- queue
```

What a caller sees:

| Kernel backend | Datapath |
|---|---|
| `Connector::connect_datagram(source, destination)` | `Connector::connect_udp(source, destination)`: a synchronous call that returns a `DatapathDatagram` (`UdpSession`, `DatagramIo`) with an ephemeral port of the client host (32768 to 60999 as Linux, one cursor per host, a 4 tuple table). It fails with `UnknownSource`, `UnknownDestination`, and `Stack` for port 0 or an exhausted host (and, since TRN-30, `InvalidInput` for addresses of two families) |
| `KernelDatagramListener::bind(address)` and `accept` | `Datapath::listen_udp(port)` returns a `UdpListener` for every server host of every lane. `accept` gives `(session, peer)` with the first datagram queued, and a closed session lets the next datagram of the peer open a new one |
| `KernelDatagram` | `UdpSession`: connected, receive queue of 64 datagrams, cut to the buffer with the full length returned, `NotConnected` after `close` or drop, `InvalidInput` above `MAX_DATAGRAM` |

The layer computes the IPv4 header checksum and the UDP checksum on send and verifies both on receive (a UDP checksum of zero means none, a computed zero goes out as `0xFFFF`). It pads frames to 60 bytes, keeps the VLAN tag of the port, and takes the MAC addresses from the neighbor tables of the pools, which are the tables of the stack and the ARP responder. A datagram above the MTU leaves as IPv4 fragments (the MTU is 1,500 and the datapath lowers it to what the queue carries). Received fragments are **not reassembled**: they are dropped and counted. There is no ICMP either way, so a send never fails with `ConnectionRefused`. The counters (`UdpStats` in `LaneStats::udp_client` and `udp_server`) say what was delivered, accepted, dropped by cause (bad checksums, malformed, fragments, no listener, full queues, not for the arm), and how often a sender waited for room in the send queue (1,024 frames).

`FrameHook` has a new method `poll_tx`, which the splitter calls on every pass of its queue, so a hook can send frames that no received frame caused. The count that the splitter's `recv_batch` returns includes them. `DatapathConfig::udp` sets the layer; `None` turns it off.

#### Tests

- `engine/fps-udp-own/src/wire.rs` and `tests/layer.rs`: the checksum, fragments, and the layer between two arms in memory (25 tests).
- `engine/fps-datapath/tests/udp_memory.rs`: two lanes with 40 echo sessions, UDP and TCP on one port, fragments, the MTU of a small queue, ports freed, settings.
- `engine/fps-datapath/tests/dns_scenarios.rs` is the acceptance. It runs both sides of every flow of the seven dns scenarios through the datapath (UDP over the UDP layer, the TCP flow of `tcp_fallback` over the own stack), checks the messages against the table of `modules/dns/tests/scenarios.rs`, and compares the bytes of both sides. Three tests: in memory (no root); over an AF_XDP queue on each end of a veth pair; and with the client side over AF_XDP and the server side over kernel sockets on the other end, which makes the kernel verify the checksums and the framing and resolve the client hosts with ARP. The veth tests need root (`sudo -n`), skip without it, and fail with `CI=true` or `FPS_REQUIRE_AFXDP=1`. The kernel's transmit checksum offload is turned off with the ethtool ioctl (TRN-26 finding).

```bash
cargo test -p fps-udp-own                                       # no root
cargo test -p fps-datapath --test udp_memory                    # no root
FPS_DNS_RESULTS=/tmp/dns FPS_REQUIRE_AFXDP=1 cargo test -p fps-datapath --test dns_scenarios -- --nocapture
```

#### Results

2 vCPU sandbox, debug build, 20 rounds (`engine/fps-datapath/results/sandbox-2vcpu-trn29/`): 140 UDP flows and 20 TCP flows, 0 failures, 0 bad checksums, 0 malformed frames, 0 drops, in 0.52 s over two AF_XDP queues and in 0.54 s against the kernel server. Both queues bound in copy mode. The numbers are the cost of the scenarios (each flow waits for its round trip), not a rate.

#### Limits

| Limit | Consequence |
|---|---|
| No reassembly | A peer that fragments its datagrams is not heard. Needs a timer and a memory budget per host |
| No ICMP | A module that waits for `ConnectionRefused` on UDP times out on this backend |
| IPv4 only | IPv6 since TRN-30, see the next section |
| Ports 67 and 68 | DHCP goes to its own hook and never reaches the layer |
| Neighbors from the pools | A client connect to an address that no pool holds fails with `HostUnreachable`. A server session answers a peer that the pools lack at the source MAC of its first frame |

### IPv6 in the own stack (TRN-30)

The own TCP stack (`fps-tcp-own`), the UDP layer (`fps-udp-own`), and the datapath (`fps-datapath`) speak IPv6 next to IPv4. A pool of the test configuration has IPv4 prefixes, IPv6 prefixes, or both (`HostPool` already allowed it, with one MAC address per host for both families), and the family of a session follows its addresses.

| Layer | What changed |
|---|---|
| `fps-tcp-own` `wire` | The 40 byte IPv6 header (no extension headers, no header checksum, hop limit 64) and the TCP pseudo header of RFC 8200 section 8.1. `parse` reads the version nibble, `parse_frame` takes both Ethernet types. |
| `fps-tcp-own` `host` | `Tuple` and `Conn` hold `IpAddr`. The MSS is the MTU minus 40 (IPv6) or 20 (IPv4) minus the 20 byte TCP header, and a peer that sends no MSS gets 536 (IPv4) or 1,220 (IPv6). A pair of two families is refused with `InvalidInput`. |
| `fps-tcp-own` `net`, `pools` | `Station` caches by `IpAddr` and sends the ethertype of the family. `Pools::pair_v6` and `sized_v6` build IPv6 pools, `HostSlice::address6`, `ip`, and `index_of` read them, and `OwnStack::connect_addrs(local, remote, port)` opens a session of either family (`connect` and `connect_hosts` use IPv4 for a pool that has both). |
| `fps-udp-own` | The UDP checksum is required (a zero checksum is a bad checksum), and a datagram above the MTU leaves as fragments with a fragment header. Received IPv6 fragments are dropped and counted, as for IPv4. `UdpArm::connect` takes the family from the addresses. |
| `fps-datapath` | `classify` has `Ipv6Tcp`, `Ipv6Udp`, and `Ipv6Other`; the splitter hands neighbor solicitations to the `Responder` of `fps-hosts`, which answers for a host of the lane with the MAC address of the stack (`nd_replied`, `nd_ignored`), so a peer resolves the hosts of the pools. |

What is not there: the stack sends no neighbor solicitation of its own (neighbors come from the pools, and the datapath resolves the next hop of a peer outside them since TRN-35, see below), reads no ICMPv6 (no packet too big, no echo), and parses no extension header (such a frame is `Ipv6Other` and dropped). A connection costs 24 bytes more and a table entry 28 more than with IPv4 only, because the tuple holds two 17 byte `IpAddr` values.

#### Tests

- `engine/fps-tcp-own/tests/own.rs`: six IPv6 tests in memory (bulk both ways, MTU 1,280 with an MSS of 1,220, loss, corruption, a dual stack pool, many tagged hosts), and unit tests of the wire format with a hand computed checksum.
- `engine/fps-udp-own/tests/layer.rs`: five IPv6 tests (request and response, MACs and ethertype and VLAN, a dual stack, fragments not heard, corrupted frames).
- `engine/fps-datapath/tests/memory.rs`: TCP and UDP sessions on two IPv6 lanes, the ND answer compared with the source MAC of a SYN, an unknown target ignored, router advertisements to the hook, and the classes.
- `engine/fps-datapath/tests/dns_scenarios.rs` runs the dns scenarios (TRN-29) and the loss workload (TRN-22) over both families, in memory and on a veth pair with an AF_XDP queue on each end. The kernel server variant gives the other end of the veth to the kernel with an IPv6 address; the kernel resolves the client hosts with neighbor solicitations that the splitter answers, and checks every IPv6 checksum. The loss workload of `fps-tcp` (`loss_transfer`) runs on the datapath through a `TcpStack` adapter of the test. A veth end that an AF_XDP socket owns bypasses the queueing disciplines, so each arm is wrapped in a queue that drops 2 percent of the received frames from a seeded generator, and the run must confirm every byte and show that the stacks sent segments again.

```bash
cargo test -p fps-tcp-own -p fps-udp-own                      # no root
FPS_REQUIRE_AFXDP=1 cargo test -p fps-datapath --test dns_scenarios -- --nocapture
```

#### Limits

| Limit | Consequence |
|---|---|
| No neighbor solicitation sent | By default a peer is a host of the pools and its MAC address comes from them. A peer behind a router is reached with `DatapathConfig::neighbors` (TRN-35, see below) |
| No ICMPv6 | No path MTU discovery (the MTU is fixed, and 1,280 is the floor), no `ConnectionRefused` over UDP |
| No extension headers, no reassembly | A packet with a hop by hop or routing header, and a fragmented datagram, are dropped |
| The kernel server test needs IPv6 in a new namespace | A kernel without IPv6 fails it under `CI=true` and skips it otherwise |
| Per connection memory | 24 bytes more per connection and 28 per table entry, for both families |

### The own stack API for the datapath (TRN-33)

TRN-27, TRN-29, and TRN-30 worked around four gaps of `fps-tcp-own`. TRN-33 closes three of them, and the datapath and the UDP layer use the new calls.

| Gap | Call |
|---|---|
| `Listener::accept` could not return the peer, because `OwnSession` did not know its addresses | `OwnSession::local_addr()` and `peer_addr()` return `io::Result<SocketAddr>` (the type of `KernelStream`, and `NotConnected` after `abort`). On the client side the local address is the client host and its ephemeral port, on the server side it is the server host that the peer reached and the listening port. `Datapath::listen` gives `Listener::accept` the shape `(session, peer)` of `KernelListener::accept` |
| A listened port stayed served for the life of the stack | `OwnStack::unlisten(port)` removes the listener as closing a listening socket does: connections that wait in the backlog (still handshaking, or established and not accepted) are reset, a task in `accept` wakes and fails with `InvalidInput`, accepted sessions are not touched, and the port can be listened on again. `Listener` of the datapath calls it on every lane when it is dropped, and `Datapath::listen` undoes the lanes that already listen when a later lane fails |
| The MAC addresses were assigned twice, once in `Pools::stations` and once in `fps-datapath::responders`, and a test pinned them against each other | `Pools::tables()` returns `Tables { client, server }`, the neighbor table of each side, claimed from one ledger (client block first). `Pools::stations()` builds the stations from it, and the datapath builds its ARP and ND responders from it. The test that compared an ARP reply with the source MAC of a SYN is gone, because there is one source; it now reads the expected MAC from `Pools::tables`, and `pools.rs` has unit tests that the stations use the MACs of the tables for IPv4, IPv6, and a tagged link |

The fourth gap, `OwnStack::input(side, frame)` and `take_tx(side)` so that a lane runs one stack per arm without the `FrameIo` wrapper, is not done: it changes how the stack is driven and is not small. The wrapper is enough for the datapath as it is.

Tests: `engine/fps-tcp-own/tests/api.rs` (six tests: the addresses over IPv4 and IPv6 and with several hosts, distinct peers, `unlisten` and listening again, a refused connect after `unlisten`, queued connections reset and a waiting accept failed, accepted sessions kept) and `tests/own.rs` (`unlisten_resets_a_handshake_that_is_not_finished`, with a link that drops everything the client sends after its SYN, so the server holds a connection in SYN-RECEIVED).

### The http1 module on AF_XDP, end to end (TRN-12)

TRN-12 closes the AF_XDP TCP path of decision record 0014. It runs the ten http1 scenarios (six in the clear, four over TLS with the browser profiles) through the agent on the AF_XDP backend and scores the capture with the realism runner, as the kernel run does. The agent is not changed: a plan with the backend `BACKEND_AF_XDP` and `fps-agent run` is the whole path (AGT-24, AGT-25).

`engine/fps-datapath/realism/http1-afxdp.sh AGENT_BINARY OUTPUT_DIR` (root) does the run. TRN-36 generalized it to every first party module (`module-afxdp.sh MODULE AGENT_BINARY OUTPUT_DIR`, see [Every first party module on AF_XDP (TRN-36)](#every-first-party-module-on-af_xdp-trn-36)), and the http1 script is now a call of that one. TRN-12 had moved the six plain scenarios to the ports 8080 to 8085, because two mix entries cannot listen on one port of one server host. TRN-36 runs them in rounds instead and keeps the port 8080 of the kernel run. The four https scenarios keep the ports 8443 to 8446 of their manifests. The load is 5 sessions per second for each entry at the default of 20 sessions per scenario (`FPS_AFXDP_SESSIONS`), and the clients and servers are the pools `10.1.R.0/24` (profile `windows11`, R is the round) and `10.2.0.0/24` (profile `linux6`).

The agent needs both arms of the datapath in one process, and AF_XDP takes the frames of its two ports before any tap, so the script builds this wire:

```text
 <p>-agent                      <p>-wire
 dpc ---------------------- wc ]
                                ] br0 (forwards only)
 dps ---------------------- ws ]
```

`tcpdump` on `wc` sees every frame of both directions, as the capture on the router's client arm does in the kernel run. Both ends of the veth pairs have the checksum offloads off, and IPv6 is off in the namespaces, so the capture holds only the traffic of the run. The script then scores the capture with the labels of `modules/http1/fixtures/expected.yaml`: the conformance check with nDPI, the parser error check (`FPS_AFXDP_PARSERS`, `tshark` by default and `tshark,zeek` in CI), the encryption check (decryption and JA4 of the four browser profiles), and `realism decrypted` for the HTTP inside TLS.

The decryption needs the TLS secrets of the run. `fps-agent run --keylog FILE` writes the secrets (AGT-27), and the script injects them into the capture with `editcap`, as `capture.sh` does. An older agent without the flag runs the same sessions, and the script prints a notice and leaves the `tls` section out of the labels, so the encryption check is not scored.

Sandbox run (2 vCPU, debug build, kernel 6.18, `engine/fps-datapath/results/sandbox-2vcpu-trn12/`): 199 client and 199 server sessions attempted and succeeded, none failed (131 http1 and 68 https in the capture), 3,316 TCP frames through the lane, both ports native in copy mode. With the key log: conformance, parser errors (tshark), and encryption pass, 68 of 68 TLS sessions decrypt and match their JA4, and the decrypted view classifies as HTTP 68 of 68. A run with 100 sessions per scenario (1,027 sessions) passed the same checks in 24 seconds. Zeek is not installed in the sandbox, so the Zeek parser ran in CI only.

`test_make_plan.py` (pytest, no root) checks the plan, the digest of the staged module tree, and the labels.

### Every first party module on AF_XDP (TRN-36)

TRN-36 generalized the http1 run of TRN-12 to every first party module that has scenarios, so each one runs through the agent forced onto AF_XDP and is scored like its kernel run. `engine/fps-datapath/realism/module-afxdp.sh MODULE AGENT_BINARY OUTPUT_DIR` (root) does it, and `http1-afxdp.sh` calls it for http1. The wire, the key log, and the scoring are those of the TRN-12 section above. These are the parts that are new.

- **Rounds.** A plan listens once for each port of a server host, and most modules run all their scenarios on one port (dns on 53, imap on 143, ssh on 22), so the scenarios do not fit one plan. `make_plan.py` groups them into rounds, a round holding scenarios until two would listen on one transport and port. The script runs the agent once for each round, and all rounds go into one capture. The labels of the kernel run (`modules/MODULE/fixtures/expected.yaml`) therefore keep their ports, and a dissector that works by port (tshark, Zeek) reads the capture as it reads the kernel capture. Each round has its own client prefix (`10.1.R.0/24`), MAC base, and seed, because the own stack numbers client ports upward from 10000 in every run and a 5-tuple that came up twice would merge two sessions.
- **Profiles.** `engine/fps-datapath/realism/profiles.yaml` holds what the `capture.sh` of each module sets: the sessions per scenario, the variables (`--var`, `--var-tls`, hls `params.json`), the moved ports (imap and smtp STARTTLS), the skipped scenarios, and the decrypted check. A module without an entry fails `test_every_first_party_module_has_a_profile`. `content` and `tls` have no scenarios, so they have no run: `content` is a function library that the mail, ftp, and hls scenarios call, and `tls` is the TLS library that the https scenarios of http1, http2, imap, and smtp exercise.
- **Sessions.** The agent starts sessions at Poisson arrivals (5 per second for each entry, for as long as the sessions of the profile need). The same seed gives the same arrivals, so the counts repeat, but they are not the counts of the kernel run. `finalize.py counts` reads the sessions that the agent started for each scenario and puts them into the labels that name a number (the instances of the structure check of strike, the streams of the rung check of hls), so the capture must still hold every session that the agent started.
- **Checks next to the score.** The kernel run of ftp, smtp, ssh, and sip_rtp also runs `structure_check.py` or `zeek_check.py`. The profile names them, and `finalize.py checks` runs them with the counts that follow from the sessions that the agent started (messages, transfers, calls). They need Zeek and are skipped without it, as in the kernel run.
- **UDP needs a moment.** The first datagrams of the first round were lost when the agent started at once after the bridge came up, and a UDP session does not retransmit. The script waits a second (`FPS_AFXDP_SETTLE`).
- **Exit code.** 0 when the agent exited with 0 in every round (the criterion `sessions.failed_ratio < 0.001` holds) and every check passes, 1 when a check or an agent run fails, 2 when the run cannot start.

Results (sandbox, 2 vCPU, root, kernel 6.18, debug build, nDPI 4.14 built by `realism/ci/build-ndpi.sh`, tshark 4.2.2 and Zeek 9.0.1 as parsers; files in `engine/fps-datapath/results/sandbox-2vcpu-trn36/`). Every row passed in three consecutive sweeps.

| Module | Rounds | Scenarios run | Sessions started and succeeded | Sessions in the capture | Run time | Checks scored |
|---|---|---|---|---|---|---|
| dns | 6 | 6 | 116 of 116 | 116 | 28 s | conformance, parser errors |
| ftp | 5 | 5 | 36 of 36 | 36 | 14 s | conformance, parser errors, structure_check.py, zeek_check.py |
| hls | 1 | 2 | 6 of 6 | 6 | 8 s | conformance, parser errors, temporal |
| http1 | 6 | 10 | 196 of 196 | 196 | 32 s | conformance, parser errors, encryption, decrypted view |
| http2 | 1 | 4 | 87 of 87 | 87 | 12 s | conformance, parser errors, encryption, decrypted view |
| imap | 5 | 6 | 46 of 46 | 46 | 16 s | conformance, parser errors, encryption, decrypted view |
| ldap | 4 | 4 | 27 of 27 | 27 | 24 s | conformance, parser errors |
| mqtt | 5 | 5 | 21 of 21 | 21 | 28 s | conformance, parser errors |
| ntp | 5 | 5 | 30 of 30 | 30 | 24 s | conformance, parser errors |
| pop3 | 5 | 5 | 36 of 36 | 36 | 14 s | conformance, parser errors |
| sip_rtp | 5 | 5 | 18 of 18 | 34 | 20 s | conformance, parser errors, temporal, zeek_check.py |
| smb2 | 3 | 3 | 22 of 22 | 22 | 10 s | conformance, parser errors |
| smtp | 3 | 7 | 57 of 57 | 57 | 14 s | conformance, parser errors, encryption, decrypted view, zeek_check.py, zeek_check.py |
| ssh | 4 | 4 | 19 of 19 | 19 | 18 s | conformance, parser errors, zeek_check.py |
| strike | 1 | 2 | 47 of 47 (24 conn_churn, 23 slowloris) | 815 | n/a | conformance, parser errors, structure (47 instances, none failed; bug #689, PR #703) |

The sessions of sip_rtp in the capture are the SIP dialogs and the RTP flows. The run time includes the time of the rounds only, not the build. A forced run of a module took 8 to 32 seconds.

#### The exclusion list

The exclusion list is empty. `dns/tcp_fallback` and `hls/vod_segments` left it with bug #688 (a session that no call claims now waits for a subscriber of its host for 2 s, `PENDING_GRACE` in `engine/fps-agent/src/transport.rs`), and `strike/conn_churn` left it with bug #689 (the own stack keeps a connection that the peer reset after the handshake and before `accept` in the accept queue of its listener, `accept` returns it, and its first read reports the reset, as a kernel does; `on_segment` in `engine/fps-tcp-own/src/host.rs`). Every first party module with scenarios runs all of them on AF_XDP, and the kernel fallback of AGT-29 now serves only the plan shapes that the backend refuses (families, VLANs, pools), not scenarios.

The exclusion list is the key `excluded` of `profiles.yaml`, and adding an entry under `excluded` takes a scenario out of the run. The agent reads the same file in `check_af_xdp` (AGT-29), so removing an entry also lets the agent run that scenario on AF_XDP; the unit test `the_exclusion_list_is_the_file_that_the_realism_run_reads` in `engine/fps-agent/src/backend.rs` pins the names (the list is empty today), and a package that adds or removes an entry updates it in the same pull request. Nothing else needs ICMP, UDP datagrams above the MTU, several pools on a side, VLANs, or DHCP: the first party scenarios use TCP and UDP datagrams below 1,500 bytes between one client pool and one server pool.

#### The CI job

`.github/workflows/trn-afxdp-modules.yml` (added by the coordinator) builds `fps-agent` once and runs one matrix job for each of the 15 modules on a hosted `ubuntu-24.04` runner, with `sudo`, the tools from `realism/ci/install-tools.sh`, and `FPS_AFXDP_PARSERS=tshark,zeek`. Each job keeps its result files as an artifact. Run one module by hand with `sudo engine/fps-datapath/realism/module-afxdp.sh MODULE target/debug/fps-agent OUT`. It needs ip, ethtool, tcpdump, editcap, ndpiReader (4.14 for http2), tshark, PyYAML, and Zeek for the Zeek checks.

`test_make_plan.py` and `test_finalize.py` (pytest, no root) check that every module has a profile, that no round listens twice on a port, that the profile variables exist, the digest of the staged tree, the labels with and without the key log, the exclusions, and the counts.

| Limit | Effect |
|---|---|
| The scenarios of a module run one round after another | Two scenarios on one port never overlap in time, as in the kernel run. A plan that mixes them still needs one port for each entry |
| The counts of sessions are Poisson | A scenario with a small count (ssh `scp_download` 2, mqtt `subscribe` 2) has few sessions in the capture. The seed fixes the count, so a run repeats |
| One lane, queue 0 | The runs use one core each, because the agent starts one lane (AGT-30) |

### DHCP leases on the datapath (TRN-37)

A host pool that leases its addresses (`HostPool::dhcp`, HST-04) runs on the datapath. Set `DatapathConfig::dhcp` (`DhcpSettings`: the `DhcpConfig` of `fps-hosts` and a lease timeout, 60 seconds by default) and give a lane a pool without a prefix. Without the settings `Datapath::start` fails with `DhcpOff`, which names dhcp4. The datapath runs one `DhcpHosts` per leasing arm and lane as the DHCP hook of the splitter (`hooks.dhcp` of that arm, over the VLAN of the pools, with a seed derived from the seed of the settings, the lane, and the side). The hook reads the DHCP frames of the splitter, sends what is due on every pass through a collecting queue that turns the frames into replies, and publishes the lease table of the clients to a `LeaseBoard` (`fps_tcp_own::leases`) once per pass. A client pool, a server pool, or both may lease.

The caller waits with `Datapath::wait_leases()` (blocking) or `Datapath::leases().wait_async()`. The phase ends when every host of every leasing arm holds an address, or at the timeout with `DatapathError::LeaseTimeout`, whose text reads `dhcp4: 0 of 3 leases bound after 400 ms (lane 0 Client pool clients: 0 of 3)`. `LeaseMonitor::boards()` returns the live `LeaseBoard` of each leasing arm as an `ArmBoard` (lane, side, `Arc<LeaseBoard>`), so a caller reads the address a leased host holds with `board.addr_of(index)` (the agent needs it as the source of a connect, AGT-23). `LeaseMonitor::report()` returns the bound count of each arm without waiting, and a datapath with no leasing pool reports an empty, complete phase. The clients go on renewing after the phase.

Everything that read an address from the prefix of the pool now reads the board, and follows its generation (a renewal for the same address does not move it):

| Reader | What it does with the lease |
|---|---|
| `Station` of the own stack and of the UDP layer (`Station::with_local_leases`, `with_peer_leases`, built by `Pools::stations_with`) | `local_mac`, `owns`, and `resolve` find a leased address in the snapshot of the board, which holds a `Responder` over `NeighborTable::add_leased`. A lost address stops being served. Leased lookups are not cached |
| `OwnStack::with_links_pools_and_leases` | `connect` by index and the next pair of hosts take the address of a leased host and skip a host with none |
| `Splitter::with_leases` | ARP requests and neighbor solicitations for a leased address are answered with the MAC address of the host, from the same MAC blocks as the stations (`Pools::blocks` is the one ledger) |
| `Route::of_leases` | The route of the arm takes its subnet from the first lease with a mask and its router from the first lease with a router option. The route is rebuilt when the generation moves, so a DHCP arm needs no gateway in the plan |
| `Connector` | A source address is a client host when a host of a lane holds it |

`HostSlice` and `Pools` stay plain data that the agent builds by hand, so the lease table does not travel inside them. It travels as `Leases` (one optional board per side, `Pools::leases()`) next to them, and `HostSlice::address_leased` and `index_of_leased` read a `LeaseTable` for a caller that has one. A pool of the lane that leases is cut into slices of hosts like any other (`split_pools`), and each lane leases its own slice, with the MAC address `mac_base + i` of the absolute index.

Tests. `cargo test -p fps-datapath --test dhcp_memory` needs no root: a router thread that is the DHCP server of the client arm and forwards between two arms carries sessions of six leased hosts on two lanes to a static server pool (every lease binds, the servers see the leased addresses, the arm resolves its router from the lease and nothing else), a leased host answers ARP for its address and for no other, a datapath with no DHCP server fails its lease phase at the timeout with an error that names dhcp4 and `0 of 3`, a leasing pool without settings is refused, and a static datapath has an empty phase. `cargo test -p fps-datapath --test dhcp_harness` is the harness test (root or `sudo -n`, `ip`, AF_XDP; `CI=true` or `FPS_REQUIRE_AFXDP=1` turn a skip into a failure): it brings up the QA-01 harness, moves the server arm interface into the client namespace as `eth1` so that one process owns both arms, runs a DHCP server in the router namespace, and a datapath whose client pool of eight hosts leases. All eight bind, and one session of 64 KiB from each leased address reaches the static server pool of four hosts through the kernel router and back, with none failed.

Limits. The hosts of one arm share one subnet and one router in the route: a lease that disagrees is not followed. A start of 20,000 hosts publishes the table once per pass and the readers build their snapshot once per generation, which costs one copy of the table per pass while leases arrive. The datapath does not release leases when it stops, and the kernel backend still refuses DHCP (the agent, AGT-23).

### Next hop resolution in the datapath (TRN-35)

The own stack answered ARP and neighbor discovery for its pools but never asked, so a peer behind a router worked only when the next hop MAC address was configured by hand (the TRN-26 harness gave the router interface the MAC address that the stack used). With `DatapathConfig::neighbors` set (`Some(NeighborConfig)`, off by default) the datapath resolves the next hop itself, over IPv4 with ARP and over IPv6 with neighbor solicitation.

How it works. The stack does not know MAC addresses of peers any more: `OwnConfig::resolve_next_hop` (which the datapath sets) makes `Station::resolve` answer `NEXT_HOP_MAC` for every peer, in the table or not, and the frame leaves the stack with that marker as its Ethernet destination. The `Splitter` of each queue keeps a neighbor cache (`engine/fps-datapath/src/neighbor.rs`) and acts on the frames with the marker, TCP and UDP alike:

| Step | What happens |
|---|---|
| Next hop | The destination itself when it lies in the prefix of the host pool of the arm (or is an IPv6 link local address), the gateway of the pool when it does not, and the destination again when the pool has no gateway (a flat link, or a router with proxy ARP) |
| Fresh entry | The frame leaves at once with the MAC address of the entry |
| No entry | An ARP request or a neighbor solicitation (RFC 826, RFC 4861) goes out from the host that sends, with the VLAN tag of the port, and the frame is held. Frames to the same next hop join the hold and send no second request. The hold is bounded per next hop (32) and in all (1,024), and the oldest frame goes first |
| Retry and failure | The request is repeated every second, 3 requests in all. Then the held frames are dropped, and the next hop is not asked for again for 3 seconds, so a dead gateway costs one round of requests and not one per frame |
| Answer | An ARP reply or request from the next hop, or a neighbor advertisement (hop limit 255 and a good checksum, RFC 4861 section 7.1.2) gives the MAC address, the held frames leave in order, and the entry lives 60 seconds. The cache creates entries only for frames of the stack, so unsolicited answers change known entries and nothing else |
| Aging | An entry that ran out is asked for again, and the frames wait meanwhile. A full table (65,536 entries) forgets what expired first and drops the frame otherwise |

`NeighborConfig` holds the numbers (lifetime, retry interval, requests, failure hold, the two hold bounds, the table size). `FrameCounters` of each arm counts `neighbor_requests`, `neighbor_answers`, `neighbor_resolved`, `neighbor_failed`, `neighbor_refreshed`, `neighbor_expired`, `neighbor_held`, `neighbor_released`, `neighbor_dropped` (with `neighbor_table_full`), and the gauge `neighbor_entries`. The cache takes its time from the clock of the stack, so tests move it with `ManualClock`.

Where the gateway comes from. `HostPool::gateway()` of the pool that the arm serves, which the plan gives for each host pool (`fps-agent` passes it through `HostPoolSlice`). `Route::of(&pool)` reads it. With the datapath resolving, `Connector::connect` also takes a destination that no pool serves (of the family of the source), because the peer is behind a router. The gap: a pool that leases its addresses with DHCP has no prefix and no gateway, and the datapath does not run DHCP yet, so such hosts resolve every destination themselves. HST-04 keeps the router of each lease (`LeaseTable::router_of`); when the DHCP hook is wired, `Route` can take the gateway from there. The agent (`fps-agent`, AGT-25) leaves `neighbors` at `None`, so a run through the agent keeps reading peers from the pools until it sets `neighbors`.

Tests (no root): `neighbor.rs` has 11 unit tests (next hop choice, request contents and VLAN, hold and release in order, retries and failure, negative cache, a late answer, aging and refresh, the hold bounds, strangers and unusable MAC addresses, the table bound, IPv6, a frame that is not IP). `tests/next_hop.rs` has 9: the splitter against a hand played neighbor (hold, release, repeats, a full queue that must not make it hold a frame twice, an IPv6 advertisement and a forged one), and the whole datapath with a router thread between the two arms of a lane that answers ARP and neighbor solicitation for the gateway of each side and forwards the rest. TCP echoes 20,000 bytes through it over IPv4, over IPv6, and with a VLAN, and a gateway that never answers fails the connect and is counted.

The acceptance test is `engine/fps-datapath/tests/router.rs` (root, `ip`, `sysctl`, AF_XDP): three namespaces, the own stack over an AF_XDP queue on `oa`, a kernel router with forwarding in the middle, a kernel socket peer behind it. The only address that the own stack knows is the gateway of its pool, and no namespace has a neighbor entry configured. Four scenarios, each a byte checked echo of 256 KiB: the own stack as client and as server, over IPv4 and over IPv6. The helper is the test binary again, started with `ip netns exec`; `FPS_ROUTER_KIB` sets the size and `FPS_ROUTER_RESULTS` names a directory for `router-<scenario>.txt`. The IPv4 scenarios ran in the sandbox: one request, one resolution, no failure, one held frame released, with the kernel router resolving the own host through the splitter when the kernel was the client. The sandbox kernel has no IPv6, so the two IPv6 scenarios skip there and run in CI, where `CI=true` turns a skip into a failure.

| Limit | Effect |
|---|---|
| No reachability probing | An entry is trusted for its lifetime and asked for again after it, not probed. A gateway that changes its MAC address inside the lifetime is found when a request of the gateway arrives (it refreshes a known entry) or when the entry ages out |
| Replies to unsolicited requests are not cached | A host that only listens does not learn its gateway until it sends |
| The router test serves one gateway per family | Several gateways, routes per prefix, and ICMP redirects are not read; the pool has one gateway |

### Interop of the own stack with smoltcp and the kernel (TRN-26)

TRN-26 runs the own stack against the two other stacks of the harness, in both directions, with a loss workload whose bytes are checked and with checks of the TCP options on the wire. The code is in `engine/fps-tcp`: `wiretap.rs` (the tap), `interop.rs` and `SmolStack::bridged` (smoltcp), `interop_kernel.rs` (the kernel), and the tests `tests/interop_smol.rs` and `tests/interop_kernel.rs`.

- **Own and smoltcp, in memory (no root).** `InteropStack` joins one own endpoint and one smoltcp host with the impairment model of TRN-22. The own stack speaks Ethernet frames and smoltcp speaks IP packets (its medium is `ip`), so the link holds Ethernet frames and the smoltcp side adds the header on send and strips it on receive. The MAC addresses come from the station tables of the own stack, so no ARP runs. The smoltcp interface has a /8 prefix, so both addresses are on link.
- **Own and the kernel, over AF_XDP and a veth pair (root).** The namespace link of TRN-22 (client, router with the `tc` qdiscs, server) carries the traffic. The own stack runs in a helper process in the client namespace over an AF_XDP queue on `vc` (it opens) or in the server namespace on `vs` (it accepts), and the kernel is a socket peer at the other end. The own stack has no ARP requests, so the router interface that faces it gets the MAC address that the own stack uses for the next hop, and the own stack answers the ARP requests of the router. The transmit checksum offload is off on every interface (see the findings).
- **The workload** is the loss workload with `LossConfig::verify`: the stream is a position dependent pattern, and the receiver checks every byte and answers `u64::MAX` after a wrong one, so a lost, repeated, or shifted segment fails the run (the plain workload counts bytes only). Every row below is a verified transfer, 20 ms in each direction where a delay is named.
- **The tap** reads every frame with the smoltcp parser, which neither the own stack nor the kernel shares. `violations()` checks that: an acceptor answers an option only when the SYN had it; the own stack offers what it is set to offer and answers what it offers and the opener offered; the MSS of the own stack is its MTU minus 40; no segment is above the MSS or the MTU; when both SYNs carry timestamps every data segment does, and when not no segment does; a window above 65,535 bytes needs the scale on both SYNs, and the own stack never advertises more than its buffer; after a loss the receiver reports holes with SACK blocks when both SYNs offered SACK, and sends none when not; no checksum is wrong and no option is malformed.

**Loss workload, in both directions.** The own stack has CUBIC and 1 MiB buffers, smoltcp 64 KiB buffers, the kernel its defaults. Rows with a delay need `sch_netem`; the hosted runner of the `rust` job of CI has it, the sandbox does not.

| Pairing | Link | Transfer | Goodput | Sender retransmissions | Result |
|---|---|---|---|---|---|
| own client, smoltcp server | clean, 20 ms | 1 MiB | 10.7 Mbit/s | 0 | complete |
| own client, smoltcp server | 1 % loss, 20 ms | 2 MiB | 2.3 Mbit/s | 21 (22 frames lost) | complete |
| smoltcp client, own server | clean, 20 ms | 1 MiB | 11.6 Mbit/s | 0 | complete |
| smoltcp client, own server | 1 % loss, 20 ms | 2 MiB | 6.3 Mbit/s | 58 (19 frames lost) | complete |
| own client, kernel server | clean, no delay | 1 MiB | 93.6 Mbit/s (hosted runner), 115 (sandbox) | 0 | complete |
| own client, kernel server | 1 % loss, 20 ms | 2 MiB | 5.15 Mbit/s (hosted runner) | 20 | complete |
| own client, kernel server | 20 Mbit/s, 32 KiB queue | 2 MiB | 3.61 Mbit/s (hosted runner), 11.7 (sandbox) | 517 (hosted runner), 41 (sandbox) | complete |
| kernel client, own server | clean, no delay | 1 MiB | 89.7 Mbit/s (hosted runner), 122 (sandbox) | 0 | complete |
| kernel client, own server | 1 % loss, 20 ms | 2 MiB | 4.50 Mbit/s (hosted runner) | 11 | complete |
| kernel client, own server | 20 Mbit/s, 32 KiB queue | 2 MiB | 19.2 Mbit/s (hosted runner), 18.7 (sandbox) | 9 (hosted runner), 105 (sandbox) | complete |

The in memory rows are one run each from the sandbox (the tests ran five times in a row without a failure); the kernel rows are one run each on the hosted runner (debug build) with the sandbox value where it ran. They are tests, not a benchmark: read a goodput as "completes", not as a rate.

**Options on the wire**, with every option on in the own stack (MTU 1500, so MSS 1460 where the peer is not smaller):

| Pairing | SYN of the opener | SYN-ACK of the acceptor | Agreed |
|---|---|---|---|
| own client, smoltcp server | MSS 1460, scale 5, SACK, timestamps | MSS 1460, scale 1, SACK | MSS, scale, SACK; no timestamps (smoltcp 0.14 has none) |
| smoltcp client, own server | MSS 1460, scale 1, SACK | MSS 1460, scale 5, SACK | MSS, scale, SACK; no timestamps |
| own client, kernel server | MSS 1460, scale 5, SACK, timestamps | MSS 1460, scale 10, SACK, timestamps | all four |
| kernel client, own server | MSS 1460, scale 10, SACK, timestamps | MSS 1460, scale 5, SACK, timestamps | all four |

Fallback rows (own stack options off, or timestamps, SACK, and scaling off in the namespace of the kernel with `sysctl`):

| Pairing | SYN of the opener | SYN-ACK of the acceptor | Agreed |
|---|---|---|---|
| own client (options off), smoltcp server | MSS only | MSS only | none, window 65,535 |
| smoltcp client, own server (options off) | MSS 1460, scale 1, SACK | MSS only | none, window 65,535 |
| own client, kernel server (kernel options off) | all four | MSS only | none, window 65,535 |
| kernel client (options off), own server | MSS only | MSS only | none |
| own client (options off), kernel server | MSS only | MSS only | none |
| kernel client, own server (options off) | all four | MSS only | none |

After the handshake: with timestamps agreed (both kernel pairings) every data segment carries them, with a payload of at most 1,448 bytes (1,460 minus the 12 bytes of the option); without agreement no segment carries them. The own stack advertises up to 1 MiB with scale 5 (smoltcp: up to its buffer, and 256 KiB with scale 3 in the large buffer tests), the receiver of the bulk sends SACK blocks only after a loss (249 segments with blocks at 1 % loss against the kernel, 187 in the other direction), and no segment of any run had a wrong checksum or an option that smoltcp could not parse.

Findings:

- A frame from a kernel socket reaches an AF_XDP queue with a partial TCP checksum (the veth keeps `CHECKSUM_PARTIAL`), the own stack drops it, and the handshake never completes. The harness turns the transmit offload off on every interface, with `ethtool` or, because the hosted runner has none, with the ethtool ioctl in the test binary. Any run of a kernel peer against an AF_XDP veth end needs it.
- The own stack against smoltcp and against the kernel agrees on every option it offers, and falls back to none cleanly.
- On a 20 Mbit/s bottleneck with a 32 KiB queue and no delay, the own client sent 517 segments again on the hosted runner (41 in the sandbox) and reached 3.6 Mbit/s against 19.4 with its options off. The own stack's recovery under tail drop at a very short round trip is slower than the kernel's; bug 652 found the cause and fixed it (see the next section). The 1 percent loss rows are not affected.

### A short tail-drop queue under SACK and CUBIC (bug 652)

With every option on, the own sender collapsed behind a 20 Mbit/s bottleneck with a 32 KiB tail-drop queue and no added delay: 3.61 Mbit/s and 517 retransmissions on the hosted runner (7.4 Mbit/s and 192 to 254 in the sandbox) against 19.4 Mbit/s with the options off. The regression test `engine/fps-tcp/tests/bottleneck.rs` reproduces it in memory, without root: own to own, 2 MiB, best of three runs each, options on against options off. Before the fix it read 5.3 against 18.8 Mbit/s, with 746 retransmissions and 7 timeouts. It needs SACK and CUBIC together, with a receive window that the flight can outgrow (window scaling): each one alone, or SACK with NewReno, ran at 18 to 19 Mbit/s.

Two causes, both in `engine/fps-tcp-own/`:

- **A lost retransmission was never found lost.** RFC 6675 counts a segment that was sent again in the pipe until it is acknowledged. When the queue is full the copy is dropped too (the sender runs at a window that is above the queue), nothing says so, and the recovery waits for the retransmission timer while its pipe has room for new data, so the flight grows to the receive window (535 KB in the trace). The scoreboard now keeps the highest sequence number sent when a range was sent again (`mark_rexmit_at`), and `detect_lost_rexmit` drops the range as soon as data sent after it, by more than two segments, was selectively acknowledged. The range is then a hole that `next_hole` offers again and the pipe stops counting. The kernel does the same (`tcp_mark_lost_retrans`). The counter `sack_lost_rexmits` in `HostCounters` counts them.
- **CUBIC reduced from the flight.** `ssthresh` was 0.7 of the flight, and the flight of a SACK recovery grows beyond the window. The next threshold was then above the window that had just lost packets, and a timeout inside a recovery restarted slow start at 0.7 of the whole grown flight (734 KB, with a path that carries 40 KB). RFC 9438 section 4.6 reduces the window: the reduction is now `0.7 * min(cwnd, flight)`, and a timeout in a recovery never raises the threshold that the recovery set (the kernel keeps it).

After the fix the same test reads 18.8 against 18.7 Mbit/s with 42 to 54 retransmissions against 36 to 39, and no timeout. The kernel interop row "own client to kernel server, bottleneck with tail drop" reads 19.2 Mbit/s with 43 to 67 retransmissions in the sandbox (7.4 Mbit/s with 192 to 254 before), and 19.4 with the own options off.

Not changed, and open:

- The kernel sends 9 segments again on this link where the own stack sends 40 to 70. The own stack does not pace and has no HyStart, so each probe of the 32 KiB queue overflows it by a few segments. Pacing in slow start, HyStart or a tail loss probe (RFC 8985) would lower it; none is in the stack.
- Against smoltcp 0.14, the own sender with options on stalls twice on this link (11.3 against 18.8 Mbit/s, two timeouts). The assembler of smoltcp holds four out of order ranges and discards the rest without reporting them in SACK blocks, so the unreported tail stays in the pipe of the sender until the timer fires. The kernel reports what it holds, so it does not show there. A time based loss detection (RACK, RFC 8985) would end the stall at the sender.

### Pacing, HyStart, RACK and tail loss probes in the own stack (TRN-34)

TRN-34 added four mechanisms to the own stack, each behind a switch of `OwnConfig` and a flag of `tcp_bench` (`on` or `off`): `pacing` (`--own-pacing`), `hystart` (`--own-hystart`), `rack` (`--own-rack`), and `tlp` (`--own-tlp`). The defaults are **RACK on, tail loss probes on, HyStart on, pacing off**, as the vetting agent decided on 2026-11-18 from the hosted runner rows below. Pacing is experimental. The code is in `engine/fps-tcp-own/src/rack.rs` (the pure parts: the send log, the RACK state, the probe timeout, the pacing gap), `cubic.rs` (`HyStart`), `recovery.rs`, `sack.rs`, and `host.rs`.

| Mechanism | What it does |
|---|---|
| HyStart | As in the Linux kernel: above 16 segments, slow start ends (`ssthresh = cwnd`) when the acknowledgments of a round arrive as a train that spans half the minimum round trip, or when the minimum round trip of the round rises by 4 to 16 ms (an eighth of the base). No effect with NewReno. 24 bytes in the congestion state |
| RACK (RFC 8985) | The send log (`TxLog`: one entry per burst, with a 32 bit microsecond time) gives each segment its send time. A segment is lost when a segment sent later was delivered and the round trip plus a reordering window (a quarter of the minimum round trip, at least 100 us) has passed. A reorder timer covers the window. The marks go into the scoreboard (`rack_lost`) and feed `pipe` and `next_hole`, so one hole with a single segment behind it is repaired without the retransmission timer. Needs SACK |
| Tail loss probe | After 2 times the smoothed round trip without an acknowledgment (with one segment outstanding, the larger of that and 1.5 times plus 200 ms; at least 10 ms; never after the retransmission timeout) the sender sends one new segment beyond the window, or the last segment again. The answer to the probe lets RACK mark the segment before it lost. A timeout clears the log and no probe goes out before something is acknowledged |
| Pacing | A segment may leave only after the gap that `2 * cwnd / srtt` in slow start and `1.25 * cwnd / srtt` in congestion avoidance give. The first flight is not paced (no round trip sample yet). A burst of up to 1 ms of data may leave at one wake, because the driver wakes in steps of about 1 ms (without this the rate fell from 52 to 18 Mbit/s on a path without loss). The pacing times sit in a separate heap, and `next_deadline` is the earlier of the two heaps |

The three timer kinds (retransmission, probe, reorder) share the one timer of a connection (`TimerKind` in the recovery box), so a connection still costs one heap entry. The log, the RACK state and the probe flags live in the recovery box and go with it when nothing is in flight.

**Reorder timer livelock (bug 687).** The row "own client to kernel server, bottleneck with tail drop, loss switches on" of the kernel interop test failed in about 1 run in 25 with "the peer did not report in time". The own client process spun at 100 percent of one CPU: `Host::expire` fires every timer that is due at one reading of the clock, and the reorder timer set itself for that same reading again, so `OwnStack::pump` never returned and held the lock of the stack. The transfer timeout of the client could not report either, because `counters()` needs the same lock. The state at the hang (read with gdb and a trace at the timer): the delivered segment ended at `snd_una` and had left at the same microsecond as the flight above it, which held one segment and the FIN behind a hole. `rack_detect` dropped the loss call `lost_to` made (it was at `snd_una`, not above) and passed `None` to `reorder_deadline`, which then took the first flight of the log as a segment that waits for its window, and it was past due. Such a flight is not older than the delivered segment, so nothing waits. `rack_detect` now passes `snd_una` as the end of what is settled, and `on_probe_timer` never sets the reorder timer for the current reading of the clock, so a wrong deadline costs a pass and not the lock. The unit tests are in `recovery.rs` (`a_flight_of_the_delivered_segments_reading_sets_no_reordering_timer`). The hang had hidden two slower endings of the same tail, each costing one retransmission timeout of 200 ms (the row then ran at 15.6 Mbit/s and failed the 90 percent check against the kernel sender, in about 3 runs in 100 after the first fix). (1) A FIN that the receiver selectively acknowledged has no entry in the send log, so `rack_on_sack` never recorded the delivery and RACK never marked the last segment behind it lost: it now takes the time of the last flight for it. (2) A lost retransmission at the tail was found only when more than two segments sent after the copy were SACKed (`detect_lost_rexmit`), which a last partial segment cannot meet: with everything that was sent acknowledged, any data sent after the copy now shows the loss (the `snd_max` argument). Before the three changes the row failed 5 times in about 80 runs in the sandbox (the hang); after them it ran 500 times with no failure, no retransmission timeout, and 19.08 Mbit/s as the slowest run.

**Memory.** `Conn` is still 176 bytes (a test asserts it; bit 15 of `opts` is the pacing flag). The scale row of the own stack in one session, main at `c1224fd` against this branch, ran twice each: 349 and 361 bytes per socket at 100,000 and 500,000 sockets on both, a difference of 0 to 1 byte. The absolute value in this session (361) is above the 263 bytes of decision record 0014, which is a property of the session and not of this package (main reads the same). The probe rate read 53,000 to 81,000 per second on main and 60,000 to 65,000 here, with an idle poll of 19 to 20 us on main and 23 to 26 us here: the extra heap look in `next_deadline` and the larger box. The idle poll is the one cost that this package adds.

**Results** (`engine/fps-tcp/results/sandbox-2vcpu-trn34/`, sandbox, 2 vCPU; the sandbox kernel sends with BBR, so the kernel rows below use `--kernel-cc cubic` or the interop test). The bottleneck is 20 Mbit/s with a 32 KiB tail-drop queue and no delay, and 2 MiB per transfer, 12 transfers each:

| Own stack | Retransmissions mean (median, max) | Goodput |
|---|---|---|
| Before (all four off) | 60 (61, 76) | 18.7 Mbit/s |
| HyStart, RACK, tail loss probes | 15 (8, 31); another series 8 to 17 | 19.07 |
| RACK and tail loss probes, HyStart off | 61 (55, 87) | 19.06 |
| HyStart alone | 16 (6, 72) | 19.0 |
| Pacing alone | 21 to 60 | 18.7 to 19.0 |
| All four | 9 to 29 (median 7 to 16) | 19.0 to 19.07 |
| Kernel with CUBIC (`tcp_bench`, 12 transfers) | 8 to 9 | 19.23 |

HyStart gives the whole reduction in retransmissions, and RACK with probes gives none there: the loss in this queue is the overflow at the end of slow start. About one run in ten does not leave slow start with HyStart (the acknowledgments are spaced by more than 2 ms, which breaks the train, and the delay test comes late), and then it sends 24 to 70 segments again, so the in-memory test (`engine/fps-tcp/tests/bottleneck.rs`, `the_loss_switches_keep_the_retransmissions_near_the_kernels`) takes the best of three: at most 18 retransmissions (twice the kernel), fewer than the run without the switches, at least 95 percent of its goodput, at most one timeout. It reads 5 against 51 retransmissions at 18.8 Mbit/s. The kernel interop test has two rows with the switches on (all four): the own client reads 19.2 Mbit/s with 5 to 6 retransmissions where it read 45 without (the kernel sender in this sandbox runs BBR and sends 126 to 164 again, so the row is not a like for like one); the test requires at most twice the kernel's retransmissions plus 8 and 90 percent of its goodput. Against smoltcp (own client, same link) the unchanged sender stalls at 11.4 Mbit/s with two timeouts in 4 of 6 runs; with RACK and probes it reads 19.0 each time (all four on: 19.2 with 5 retransmissions). That closes the second open finding of bug 652.

**HyStart is on by default.** The vetting agent turned it on (as Linux does with CUBIC) because the acceptance of TRN-34 holds only with it: without it the own stack retransmits about 60 segments in the short queue where the kernel retransmits 10. It costs nothing on the fat row and cuts the retransmissions of the fairness row tenfold. The fairness row passes its bar of 0.9 on the mean but not in every run: four hosted means of six runs each read 0.908, 0.899, 0.927 and 0.893 (0.907 over all 24 runs), and single runs dip to 0.867. With HyStart off the mean was 0.954, and single runs also dipped below 0.9 (0.836). The gap to the kernel (Jain 0.97 to 0.99, about 5 to 12 retransmissions per run against about 700 to 870) is bug #685, which the review before TRN-31 reads.

**Pacing is experimental and off.** It showed no retransmission gain. In the sandbox it raised the in-memory default loss row (61.7, 25.8, 3.10 to 78.1, 50.3, 5.01 Mbit/s with all four switches on), which nobody can explain yet and which the in-memory link may flatter, so no benefit is claimed for it (bug #686). On the fat row (0.1 percent loss, x24, 100 ms) pacing read 17.3 to 17.7 Mbit/s against 18.1 to 19.1 without; the difference is about the run to run spread (18.1 to 19.1 for the unchanged stack), but it does not reduce retransmissions at the bottleneck either. It does raise the default loss row of `tcp_bench` (0, 0.1, 1 percent at 20 ms: 61.7, 25.8, 3.10 Mbit/s before, 78.1, 50.3, 5.01 with all four on); the cause is not clear, and the in memory link may flatter a sender that does not burst. The default loss row, the fat row and the fat path with RACK and probes are otherwise unchanged by the defaults (52.6, 25.9, 3.12 against 52.7, 25.8, 3.11 in one session).

**Hosted runner rows** (`tcp-bench`, `job: loss`, `ref: wp/TRN-34`, ubuntu-24.04, 2026-11-18). The rows with the shipped defaults ran with no own-stack flags after HyStart was turned on (runs 44395339113, 44396108709, 44396841781, 44398159732, 44400587962, 44401870930); the HyStart-off column is from the runs before the switch (44386899462, 44389973366, 44392126469).

| Row | Own stack, shipped defaults | Own stack, HyStart off | Kernel |
|---|---|---|---|
| Short queue: 20 Mbit/s, 32 KiB queue, 0 ms, 2 MiB x12 (`--kernel-cc cubic`) | 9.8 retransmissions, 19.08 Mbit/s | about 60 (sandbox) | 10, 19.23 Mbit/s |
| Fat: 50 Mbit/s, 50 ms, 0.1 percent loss, 8 MiB x6 | 20.17 Mbit/s, 7.8 retransmissions | 19.79 Mbit/s, 8 | 12.97 to 21.97 Mbit/s across runs |
| Fairness: four flows, 1 Gbit/s, 20 ms, 4 MiB queue, x6 | 891 Mbit/s, Jain 0.899, 0.927, 0.893 (and 0.908 with the switch flag), about 740 to 870 retransmissions | 891 Mbit/s, Jain 0.954, about 6,800 retransmissions | 954 Mbit/s, Jain 0.97 to 0.99, 5 to 12 |
| Default loss row: 20 ms, 0, 0.1 and 1 percent loss | 53.5, 19.9, 2.89 Mbit/s | 53.1, 19.8, 2.88 Mbit/s | 52.7 to 71.7, 40.1 to 43.0, 3.76 to 4.71 Mbit/s |

The defaults change nothing on the default loss row; the own stack stays behind the kernel there at 0.1 and 1 percent loss, as before TRN-34.

### Decision record 0014 and the one session run

TRN-10 proposes decision record 0014 (`docs/decisions/0014-user-space-tcp-stack.md`), which chooses the own stack with loss recovery added for the AF_XDP backend, keeps smoltcp as a reference and a fallback, and leaves F-Stack to the parked TRN-13. Its numbers come from `engine/fps-tcp/results/sandbox-2vcpu-trn10/`, where the three stacks ran in one session on one sandbox, including the million socket rows of the own stack and smoltcp. The kernel row at a million sockets is not measured, because the open files limit of the sandbox is 20,000 and cannot be raised, so the kernel stops at 19,815 sockets. The fat-nuc numbers are still pending, and the record says when to revisit it.

### Limits

| Limit | Consequence |
|---|---|
| The kernel stack runs on the loopback interface, and a user space stack on an in memory link, and neither has delay, loss, or reordering | The numbers show stack and adapter cost only, and retransmission and congestion control stay unmeasured |
| Both hosts of a pair share a process and a core | A rate counts the work of the client and of the server, so it is not the rate of one agent talking to a remote peer |
| Scaling runs on one thread | The poll cost per socket is a per core cost, and the multi core rate comes from `--threads` on the rate workload |
| The open files limit caps a kernel run | The sandbox ends near 20,000 sockets. A host test needs a limit above 1,000,100 for 1,000,000 sockets |
| The fat-nuc run of smoltcp needs a Rust toolchain for the relay account | The smoltcp results are from the sandbox only. The owner installs a stable toolchain, and then the host test of `docs/handoff/TRN-20.md` runs through the relay |
| smoltcp walks its socket set on every poll and every frame | One socket set should hold a few thousand sockets, so a million sockets need sharding and a flow table of our own |
| smoltcp runs with TIME-WAIT skip by default, with software checksums, and with Nagle off | The rate of smoltcp is an upper bound for a stack that holds tuples in TIME-WAIT. `--smol-time-wait keep` shows the other case |
| The own stack runs CUBIC (or NewReno) on the scoreboard, and its window is bounded by its buffers (1 MiB by default) | A path with a bandwidth delay product above 1 MiB needs larger buffers (`--own-window-kib` and `--own-sndbuf-kib`). HyStart is on by default (TRN-34); `--own-hystart off` makes slow start leave only on a loss. There is no PRR (a recovery sets the window to the threshold at once), so after a loss the sender pauses for about 0.3 round trips |
| The own stack measures its rate on one thread with a driver pass per wake up | The rate of one shard is latency bound. `--threads` runs one stack per thread, and the real multi core rate of an engine belongs to TRN-12 |
