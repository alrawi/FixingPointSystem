## [2026-11-03] ingest | ENG-08 Session scheduler

We added the crate `engine/fps-sched` with constant and Poisson arrivals, a concurrency cap with a defer and a skip policy, hold, linear, and step phases, and a schedule iterator that a worker drains with an injected clock. Scheduled times depend only on the configuration, the run seed, and the stream. We added a section on the scheduler to [Engine](Engine.md). The evidence is `docs/handoff/ENG-08.md` and `cargo test -p fps-sched`, which runs the determinism, KS, phase boundary, property, and contract tests.

### Shared page notes

1. The Crates table of [Engine](Engine.md) can gain a row for `fps-sched` with the contents "Load profile, session schedule, drain iterator, and injected clock" and the next package ENG-10.
2. The `session_rng` example in the Rust section of [Coding Standards](Coding-Standards.md#rust) builds the state as `seed << 64 | session_id`. `Pcg64Mcg::new` forces the two lowest state bits to 1, so session ids 0 to 3 get the same stream. The scheduler shifts the stream above those bits, and the example should do the same or hash the id.
