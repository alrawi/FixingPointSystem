# Transport

This page describes the transport backends of the TRN workstream, which implement the `SessionIo` and `DatagramIo` seams of [Engine](Engine.md) over real networks. TRN-01 created the first backend, the kernel TCP backend in `engine/fps-backend-kernel`. Later TRN packages add kernel UDP, operating system profiles, TLS, and the AF_XDP and DPDK backends that the [Implementation Plan](Implementation-Plan.md#target-repository-layout) lists.

## Crates

The workspace glob in the root `Cargo.toml` picks up every directory under `engine/`, so a new backend crate needs no edit there. The table lists the backend crates that exist today.

| Crate | Contents | Next package |
|---|---|---|
| [`fps-backend-kernel`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-backend-kernel) | `Connector`, `KernelListener`, and `KernelStream`, which implement `SessionIo` over kernel TCP sockets | TRN-02 adds kernel UDP, TRN-03 the operating system profiles |

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

`SocketOptions` has four fields, and the same type configures clients and listeners.

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
