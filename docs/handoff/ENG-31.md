# Handoff ENG-31 A transport seam in the runtime

| Field | Value |
|---|---|
| Issue | #661 |
| Branch | `wp/ENG-31` |
| Pull request | #666 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-e31a7c, lease until 2026-11-18T14:44:34Z |
| Last verified commit | edc532b |

## Goal

The workers of `fps-runtime` open sessions only through `fps_backend_kernel` (`Connector` and `KernelListener` for TCP, the datagram calls for UDP), so a forced AF_XDP run stops with `Unimplemented` (AGT-24 finding). Add a `Transport` trait with connect, listen, and the UDP calls, each returning boxed `SessionIo` or `DatagramIo`, implement it for the kernel backend, and let `WorkerConfig` carry an `Arc<dyn Transport>`. Server workers take one listener per port. The datapath is not wired into the agent here (AGT-25). The kernel backend behaves as before.

## Plan

- [x] 1. Claim commit and handoff file
- [x] 2. The trait, the session types, and the kernel implementation (`transport.rs`, `kernel.rs`)
- [x] 3. Move the TCP workers, the data ports, the instances, and the UDP ports onto the trait; keep the existing tests passing
- [x] 4. Fake transport test, Engine page, log entry, full checks, pull request, CI

## Done and verified

- `cargo test -p fps-runtime`: unit tests (23) and every integration test pass over the kernel transport, with `tests/transport.rs` (2, the fake in-memory transport) included.
- `cargo test -p fps-agent`: all test binaries pass (113 unit tests and the integration tests; the root and AF_XDP tests skip without sudo).
- `cargo clippy -p fps-runtime -p fps-agent --all-targets -- -D warnings` and `cargo fmt --check` are clean.
- `plan.py check`, `render --check`, and `lint` pass.

- PR #666: all 30 check runs of edc532b succeeded, among them `rust`.

## Remaining

Nothing.

## Next action

The coordinator reviews PR #666.

## Blockers and open questions

none

## Decisions taken in this package

- The root of `fps_runtime` already exports `Transport` (the `fps_wasm` enum of TCP and UDP). The trait lives in the public module `fps_runtime::transport`, so the existing export keeps its meaning.
- `WorkerConfig::transport` is an `Option<Arc<dyn Transport>>`. `None` builds the kernel transport from `ClientConfig::connect` and `ServerConfig::config`, so the kernel configuration keeps its meaning and the `WorkerConfig` literals change by one line.
- A session boxes the I/O and carries its addresses, because the capture tap and the instances need them and a `Box<dyn SessionIo>` has none.
- The kernel UDP listener keeps the shared socket reader that the runtime had, behind the trait, and not `KernelDatagramListener`. It sends the replies from the shared socket, so the packets on the wire do not change. The port task counts open sessions with a wrapper in the runtime, which any transport can use.
- A server worker listens once for each port with all its addresses of the port (`listen_addrs` keeps the configuration order), and once for each address of port 0.
- `RuntimeError::Listen` holds an `io::Error` with the `BackendError` as cause, and `ClientOpener::new` takes `&dyn Transport`. Nothing outside the crate used either.

## Pitfalls

- Workers run on threads of their own, so a test that records the calls of a shared fake transport must not assume an order between workers.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-ENG-31 cargo test -p fps-runtime
CARGO_TARGET_DIR=$PWD/target-ENG-31 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T12:55Z | claude-e31a7c | claim | 33ea43e | claimed #661 with card 6979070048 |
| 2026-11-18T13:25Z | claude-e31a7c | checkpoint | 43bbc88 | the seam, the kernel transport, the workers moved, the fake transport test |
| 2026-11-18T13:50Z | claude-e31a7c | checkpoint | edc532b | clippy fixes, handoff, PR #666 opened |
| 2026-11-18T14:15Z | claude-e31a7c | review | see PR | CI green at edc532b, moved to review |
