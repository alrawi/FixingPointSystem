# Transport

This page describes the transport backends of the TRN workstream, which implement the `SessionIo` and `DatagramIo` seams of [Engine](Engine.md) over real networks. TRN-01 created the first backend, the kernel TCP backend in `engine/fps-backend-kernel`. Later TRN packages add kernel UDP, operating system profiles, TLS, and the AF_XDP and DPDK backends that the [Implementation Plan](Implementation-Plan.md#target-repository-layout) lists. TRN-04 added the TLS wrapper, see [TLS module](#tls-module).

## Crates

The workspace glob in the root `Cargo.toml` picks up every directory under `engine/`, so a new backend crate needs no edit there. The table lists the backend crates that exist today.

| Crate | Contents | Next package |
|---|---|---|
| [`fps-backend-kernel`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-backend-kernel) | `Connector`, `KernelListener`, and `KernelStream`, which implement `SessionIo` over kernel TCP sockets, the operating system profiles of TRN-03, and `KernelDatagram`, `KernelDatagramListener`, and the two datagram openers of TRN-02, see [Kernel UDP backend](#kernel-udp-backend) | TRN-04 added TLS, see below |
| [`fps-module-tls`](https://github.com/alrawi/FixingPointSystem/tree/main/modules/tls) | `TlsConnector`, `TlsAcceptor`, and `TlsSession`, which wrap any `SessionIo` in TLS 1.2 or 1.3, the client and server profiles, `RunAuthority`, and `NssKeyLog`, see [TLS module](#tls-module) | TRN-05 added the browser profiles, see [Browser profiles and JA4](#browser-profiles-and-ja4), and TRN-15 the mail and h2 profiles, see [Mail and h2 profiles](#mail-and-h2-profiles) |

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
| TCP option order and padding | `tcp_options_write` writes a fixed order, for example mss, sok, ts, nop, ws, so no profile other than linux6 matches its reference order | none |
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
