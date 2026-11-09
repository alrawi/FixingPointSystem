## [2026-11-09] ingest | ENG-19 capture TCP sessions in the runtime

`fps-capture` now owns the capture cell as `CaptureCell`, with the same behavior as the cell that AGT-05 built in `fps-agent`: one atomic flag and an `RwLock<Option<Capture>>`, so a read costs one atomic load while no capture runs. `fps-runtime` takes the cell in `RuntimeConfig::capture` (with `with_capture` to share one) and returns it from `RuntimeHandle::capture_cell()`. Every worker wraps the opener of each session in a `TapOpener`, which reads the cell once when the session opens and wraps the kernel stream in `Tapped`, so client and server TCP sessions are recorded from the moment `set` runs. We chose the opener over a change in `fps-interp` because the interpreter stays free of capture. The opener asks the socket for its addresses only while a capture runs. The evidence is `engine/fps-runtime/tests/capture.rs`: a capture that starts one second into a three second run holds only sessions that opened later, and each one reassembles into a complete `web_get` session with the independent reader of the `fps-capture` tests. The same test lets tshark parse the ring where it is installed. The [Engine](../Engine.md#capture-of-tcp-sessions) page has the details.

The agent has not switched yet. Replacing the agent cell takes two lines in `engine/fps-agent/src/control.rs`: remove the `CaptureCell` definition and write `pub use fps_capture::CaptureCell;`, and give the TCP runtime the same cell with `cfg.capture = cell.clone()` (or `RuntimeConfig::with_capture`) before `Runtime::start` in `src/run.rs`. The `tap_datagram` method of the shared cell has the signature that the UDP workers call now.

Findings:

- `fps-runtime` now depends on `fps-capture`, so `Cargo.lock` gains one dependency line.
- A process that runs a client and a server runtime over one `Capture` records each connection twice, once per side and under two session ids. The agent runs one role per process.
- tshark could not run in the sandbox of this package, so the tshark half of the test ran only in CI.

### Shared page notes

- CI: the `eng-capture` workflow runs only `cargo test -p fps-capture --test capture`. Add `cargo test -p fps-runtime --test capture -- --nocapture` with `FPS_REQUIRE_CAPTURE_TOOLS=1` to the same job, so a missing tshark fails the runtime capture test too.
- Testing: `engine/fps-runtime/tests/capture.rs` runs for about 5 seconds and includes `engine/fps-capture/tests/common/mod.rs` by path.
- AGT-05 log entry, gap 1 is closed by this package once the agent switches to the shared cell.
