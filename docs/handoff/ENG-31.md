# Handoff ENG-31 A transport seam in the runtime

| Field | Value |
|---|---|
| Issue | #661 |
| Branch | `wp/ENG-31` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-e31a7c, lease until 2026-11-18T14:20:55Z |
| Last verified commit | none |

## Goal

The workers of `fps-runtime` open sessions only through `fps_backend_kernel` (`Connector` and `KernelListener` for TCP, the datagram calls for UDP), so a forced AF_XDP run stops with `Unimplemented` (AGT-24 finding). Add a `Transport` trait with connect, listen, and the UDP calls, each returning boxed `SessionIo` or `DatagramIo`, implement it for the kernel backend, and let `WorkerConfig` carry an `Arc<dyn Transport>`. Server workers take one listener per port. The datapath is not wired into the agent here (AGT-25). The kernel backend behaves as before.

## Plan

- [x] 1. Claim commit and handoff file
- [ ] 2. The trait, the session types, and the kernel implementation (`transport.rs`)
- [ ] 3. Move the TCP workers, the data ports, the instances, and the UDP ports onto the trait; keep the existing tests passing
- [ ] 4. A worker over a fake in-memory transport, Engine page, log entry, full checks, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything after step 1.

## Next action

Write `engine/fps-runtime/src/transport.rs`.

## Blockers and open questions

none

## Decisions taken in this package

- The root of `fps_runtime` already exports `Transport` (the `fps_wasm` enum of TCP and UDP). The trait lives in the public module `fps_runtime::transport`, so the existing export keeps its meaning.

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-ENG-31 cargo test -p fps-runtime
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T12:55Z | claude-e31a7c | claim | none | claimed #661 with card 6979070048 |
