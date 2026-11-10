## [2026-11-10] ingest | ENG-27 Dropped counts of a capture

`fps-capture` counted two different events in `sessions_dropped_queue`: a session that ended while the writer queue was full (`TrySendError::Full`), and a session that ended after the writer had gone, which is every session still open when `Capture::stop` ran (`TrySendError::Disconnected`). AGT-18 showed that the second one made `run_control` flaky.

`CaptureStats` now has `sessions_dropped_overflow` for the first and `sessions_cut_at_stop` for the second. `sessions_dropped_queue` stays, as the sum of the two, because `engine/fps-agent` adds it to `sessions_dropped_oversize` for `StopCaptureResult.dropped` and that crate is outside this package. The writer also counts the sessions that were queued behind the stop job in `sessions_cut_at_stop`, where it dropped them uncounted before. A session that is queued in the instant between that drain and the end of the writer thread is still not counted. `Capture::stop` returns its counters before open sessions end, so a caller reads `Capture::stats` after them to see `sessions_cut_at_stop`.

Tests with literal values: a queue of 2 that nobody reads gives 3 overflow drops of 5 sessions and none cut (unit test), 3 sessions open at the stop give 3 cut and no overflow with the 2 earlier sessions written (component test), and the existing 500 session burst asserts that every session is written or an overflow drop.

### Shared page notes

- Contract follow-up (needs a CON package and the owner): `StopCaptureResult.dropped` (`contracts/proto/fps/agent/v1/agent.proto`, "the number of packets that the capture missed") and `dropped` of `CaptureFile` (OpenAPI) still carry overflow plus oversize plus cut at the stop, and they count sessions, not packets. To expose the split, a contract change would add fields, for example `dropped_overflow` and `cut_at_stop`, or narrow `dropped` to loss of the writer, and a follow-up in `engine/fps-agent/src/control.rs` would then read `sessions_dropped_overflow + sessions_dropped_oversize` and `sessions_cut_at_stop` apart. Until then the agent could stop a capture at rest, as `run_control` does since AGT-18.
