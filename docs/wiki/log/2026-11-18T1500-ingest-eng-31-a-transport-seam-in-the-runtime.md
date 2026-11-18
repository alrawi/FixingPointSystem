## [2026-11-18] ingest | ENG-31 a transport seam in the runtime

ENG-31 puts a `Transport` trait between the workers of `fps-runtime` and the network, so a run can open its sessions on something other than kernel sockets. It edits only `engine/fps-runtime/`, its handoff, this entry, `Engine.md`, and the mechanical `transport: None` line of the `WorkerConfig` literals in `engine/fps-agent/src/assemble.rs` and `engine/fps-agent/src/udp.rs`. It does not wire the datapath into the agent (AGT-25).

- `fps_runtime::transport` (new, public) holds the trait `Transport` with `connect`, `listen`, `connect_datagram`, and `listen_datagram`, the accept side `StreamListener` and `DatagramListener`, and the session types `StreamSession` and `DatagramSession`. A session boxes a `SessionIo` or a `DatagramIo` and carries its two addresses, which the capture tap and the instances read. The trait is not at the root of the crate, where `fps_runtime::Transport` already is the TCP or UDP enum of the extension host.
- `fps_runtime::KernelTransport` (new, `kernel.rs`) is the kernel implementation. TCP goes through `Connector` and `KernelListener`, client UDP through `Connector::connect_datagram`, and the UDP listener is the port reader that `datagram.rs` held before (one socket, one session per peer, replies from the shared socket), moved behind the trait without a change of behavior.
- `WorkerConfig::transport` is an `Option<Arc<dyn Transport>>`. `None` builds the kernel transport from `ClientConfig::connect` or `ServerConfig::config`, so the kernel path is as before. The workers, the data ports, the datagram ports, the instances, and `ClientOpener` no longer name a kernel type.
- A server worker listens once for each port with all its addresses of that port, and once for each address of port 0. `listen_addrs` keeps the order of the configuration.
- `RuntimeError::Listen` now holds an `io::Error` in place of a `BackendError`; the kernel keeps its `BackendError` as the cause, so the message chain is unchanged. `ClientOpener::new` takes `&dyn Transport` in place of `&Connector`. Nothing outside the crate used either.

Evidence: `tests/transport.rs` runs a client and a server worker over `FakeNet`, a transport in memory that opens no socket: one listen call for two hosts of one port, `listen_addrs` in configuration order with the port that the fake chose for port 0, 20 `web_get` sessions from hosts that no interface has with equal client and server byte counts, and a refused listen that fails the start with the error of the transport. The existing runtime and agent tests pass over the kernel transport. They changed only by one `transport: None` line in each `WorkerConfig` literal and by the constructor argument of the unit tests of `DataPorts`, `DatagramPorts`, and `ClientOpener`.

Findings:

- **The datapath cannot serve two server workers of one port through `Transport::listen` alone.** `Datapath::listen(port)` serves a port once for the life of the datapath, and the agent starts one server worker per plan worker, each with its slice of the server hosts on the same port (`assemble.rs`). AGT-25 therefore needs a `Transport` of its own for the datapath that listens once for each port, shares the `Listener` among the workers (or hands each accepted session to the worker whose host range holds `session.local_addr()`), and keeps a port that a data port listened on before. The data ports of the instances (`data_ports.rs`) bind a port when a waiter comes and release it when the last waiter leaves, and a second bind of the port fails on the datapath, so the datapath transport has to keep such a listener open for the life of the run, or AGT-25 leaves non entry TCP flows to the kernel.
- **UDP servers listen one address per call.** `KernelTransport::listen_datagram` takes exactly one address, and the runtime calls it so. The datapath serves `listen_udp(port)` for all server hosts, so its transport also has to sort sessions by `local_addr()` to the address that a worker or a flow accepts on. `DatagramPorts` keys its ports by address, so a session of another host of the port would not meet its waiter until that is done.
- `Transport::connect_datagram` is synchronous and `connect` is a boxed future, as `Connector::connect_udp` and `Connector::connect` are.

### Shared page notes

- Transport page, section "Backend selection": no change. The note of the AGT-24 entry about the paragraph "Nothing carries TCP over AF_XDP before TRN-12" stands; after AGT-25 it should say that the agent passes the datapath through `fps_runtime::transport::Transport`.
- Implementation Plan and sidebar: nothing.
- Testing page: nothing new. `tests/transport.rs` runs in the `rust` job with the other runtime tests and needs no root and no network.
- Module development: nothing changes for module authors.
- `Cargo.lock` does not change. No root `Cargo.toml` edit.
