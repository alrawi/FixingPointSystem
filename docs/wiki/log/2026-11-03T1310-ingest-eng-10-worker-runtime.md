## [2026-11-03] ingest | ENG-10 Worker runtime

We added the crate `fps-runtime` and the section [The worker runtime](Engine.md#the-worker-runtime) at the end of the [Engine](Engine.md) page. The runtime runs one single threaded Tokio runtime per worker thread with optional core pinning through `core_affinity`, keeps a shared nothing session model, backs the `fps-sched` clock with the Tokio timer, and sends one cumulative statistics snapshot per worker and second to a collector. A stop lets open sessions finish within the ramp down time and resets the rest, and an abort resets every session at once. A loopback test drives the http1 `web_get` example at a fixed rate with client and server workers and checks that both sides report matching counters. The evidence is `cargo test -p fps-runtime` and `docs/handoff/ENG-10.md` on the branch `wp/ENG-10`.

### Shared page notes

- [Engine](Engine.md) could add the row `fps-runtime` to the crate table, with the contents "Worker threads, optional core pinning, the Tokio clock of the scheduler, statistics snapshots, stop and abort" and the next package AGT-01, which builds a `RuntimeConfig` from the agent plan.
- [Testing](Testing.md) could name `fps-runtime` among the crates of `test-eng`, which picks it up without an edit to `mk/eng.mk`.
- The session metrics of `contracts/metrics/catalog.yaml` carry no `side` label, so an agent that runs client and server workers of one flow must keep the two sides apart. The collector merges per side today, and a CON package could decide whether the catalog needs the label.
