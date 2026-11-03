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
