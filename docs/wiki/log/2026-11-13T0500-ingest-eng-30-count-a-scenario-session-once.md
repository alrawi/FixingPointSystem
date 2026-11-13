## [2026-11-13] ingest | ENG-30 Count a scenario session once, with the bytes of its spawned flows

The engine now counts the meaning of CON-31. A scenario instance is one session of its entry flow (`flows[0]`), in the series that carry the labels of the entry flow. Every other flow side of the instance, spawned or ordered by `after`, adds its bytes to that session and fails it once, with the reason of the first side that failed.

Changes in `engine/fps-runtime`.

- `src/instance.rs`: only the entry flow calls `started`. `finish` takes the outcome of `ScenarioInstance::run` (the reason of the first failure), sums the bytes of every `SideRecord`, takes `established` from the entry flow, and records one `ended_with` for the instance. The duration runs from the start of the entry flow to the drop of the opener of the last side. A side that left no record fails the session with `internal`, as before. The sides that a failure aborts add no `aborted` count.
- `src/worker.rs`: the per flow series (`flow_metrics`, `metrics_of`) are gone. A worker registers one `SessionMetrics` with the module of the entry flow, and `Worker::metrics` returns it. `is_instance` reads a flag that is true when the scenario has more than one flow. A scenario with one flow runs the code that it ran before.
- Tests that counted flows as sessions now count sessions: `tests/instances.rs` (all six tests that read `sessions.*`), `tests/datagrams.rs`, `tests/tls_sessions.rs`, `engine/fps-agent/tests/first_party.rs` (ftp `ok >= 1`, sip_rtp `ok >= 2`), and `engine/fps-agent/tests/controller_e2e.sh` (ftp `> 0` and `-ge 1`, sip_rtp `> 1` and `-ge 2`, hls `vod_segments` at least 1). `instances.rs` and `datagrams.rs` hold two constants (`INSTANCES` or `calls` for the engine counters, `FLOW_CONNECTIONS` for the capture, which still counts a connection for each flow). The `after` test now gives its second flow a 200 ms delay and checks that the client holds two duration samples, one for each instance, and that none is shorter than that (the old code held four samples, two of them short). The failing flow test checks that no `aborted` count appears and that the client and the server fail once. The ftp test checks that the bytes of the data flows sit under `application=ftp`.
- `docs/wiki/Engine.md`: the Series row of Multi flow scenario instances, the paragraph on failures and `sessions.concurrent`, and the test rows that named 32 sessions, eight sessions, or two sessions per instance.

Decisions.

- The duration of the session ends at the drop of the last opener, as the duration of a side did before, which is the end of the last action of any flow.
- `Worker::new` no longer registers a series for the modules of the other flows. They counted nothing under the new rule and would have shown as empty series.
- The reason of a failed session comes from the result of `ScenarioInstance::run`, which holds the first failure, and not from the first failed record. They are the same failure except for the guard in `ScenarioInstance::spawn` (an invalid spawn), which fails the instance with no record.

Evidence: `cargo test --workspace` (1487 passed, 0 failed), `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all -- --check` pass locally. The controller script `controller_e2e.sh` needs the controller and root namespaces, so only CI runs it.

### Shared page notes

- [Module Development](Module-Development.md), section `expected_bytes`, paragraph "**What the run counts.**": the engine now counts this way, so the two interim sentences go ("The engine counts this way from ENG-30. Until then it reports every spawned or `after` flow as a session of its own, which halves the measured bytes per session of a scenario like `ftp/retr_epsv`."). The exact replacement for the whole paragraph is:

  > **What the run counts.** The contract defines one session for each start of the entry flow, with `bytes.tx` and `bytes.rx` holding the bytes of all its flows under the labels of the entry flow, so the controller measures bytes per session in the same unit as your estimate. A scenario whose flows counted as sessions of their own needs no new estimate, because `expected_bytes` always meant the bytes of the whole session.

  The other three edits of the CON-31 entry (Flows, the row `session.id`, Spawned flows) are unaffected by this package.
- [Testing](Testing.md): none. The counts of the tests of `engine/fps-runtime` and `engine/fps-agent` changed with the engine, and no test was added or removed.
- [Gates](Gates.md): none for this package. CTL-49 changes the paragraphs on the byte basis after this merges.
- [Implementation Plan](Implementation-Plan.md) and [Home](Home.md): none.
