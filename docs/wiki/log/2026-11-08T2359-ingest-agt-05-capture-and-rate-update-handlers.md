## [2026-11-08] ingest | AGT-05 capture and rate update handlers

The agent in `engine/fps-agent/` now serves the three v0.2 commands of CON-05. `UpdateRates` maps each entry to a total for the whole agent and applies all of them or none, across two registries: the `RateControl` that `fps-runtime` fills for TCP client workers, and a registry of its own that the UDP workers of `src/udp.rs` fill, so UDP entries are driven too. `assemble.rs` sets `FlowRef::entry` to the mix entry id. `StartCapture` builds an `fps-capture` ring that samples every session and puts it into a cell that the workers read when a session opens, so a capture that starts mid run reaches them. `StopCapture`, a limit, and the end of the run end a capture. The agent joins the ring files, oldest first, into one pcapng file, which is valid because each ring file is a complete section, and reports the size, the packet count, and the SHA-256 of that file. The file stays on the agent at a path that the controller computes from the plan id, the capture id, and the agent's capture directory, because CON-05 defines no transfer. The [Agent](../Agent.md#run-time-rates-and-captures) page has the details. The evidence is the component test `tests/run_control.rs`, where a fake controller changes the rates of a TCP and a UDP entry, starts a capture in the middle of the run, stops it, and reads the file back with its digest and packet count checked.

Findings:

- TCP sessions are not captured. `fps-runtime` opens its sessions with the kernel stream type and offers no tap hook, so the cell reaches the UDP workers only. A follow up in `fps-runtime` must read `CaptureCell` (or an equivalent shared cell) when a session opens and wrap the stream with `Capture::tap`.
- `StartCapture` refuses a non empty `filter` (`INVALID_ARGUMENT`), a named `port` and a non zero `snap_length` (`UNIMPLEMENTED`). `fps-capture` samples whole sessions and has no pcap-filter compiler, no per port tap, and no snap length. CTL-14 (filtered captures) must expect these errors until an engine package adds them.
- `StopCaptureResult.dropped` counts dropped sessions and not packets, and a capture that fills its ring (64 MiB by 8 files by default) loses its oldest sessions.
- The agent runs one capture at a time.
- The pre existing ENG-15 follow up for `src/udp.rs` (register series with the `side` label through `SessionCounters`, `Reason` mapping) is not part of this package and stays for AGT-07.

### Shared page notes

- Engine, open points of the capture section: point 1 is half done. The agent builds the `Capture` from the command and keeps a shared cell, which only the UDP workers of the agent read. Point 3 is settled by AGT-05: the digest covers the concatenation of the ring files, one pcapng file. Point 2 is unchanged.
- Engine, run time rates: the UDP workers of `fps-agent` now register their rate handles in `UdpRuntime::rate_control()`, so the gap that the section names is closed.
- Testing: `tests/run_control.rs` runs for about 25 seconds, because it measures session rates over half second snapshot windows. Its tolerances are wide (a paused entry below 4 per second, a resumed one above 60 of 120).
- CON-05 follow up for the owner: define how the controller fetches a capture file. Until then the file stays on the agent.
