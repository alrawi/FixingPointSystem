## [2026-11-03] ingest | HST-01 Host pool allocator

We created the crate `fps-hosts` under `engine/` and the [Hosts](Hosts.md) page, which records the mapping from host index to IPv4, IPv6, and MAC address, the four rules of slicing by agent and by worker, and the three host selection modes. Property tests show that slices by agent and then by worker are disjoint and cover the pool. A contract test reads every example test configuration and plan under `contracts/examples/`. We linked the page from [Home](Home.md) and the sidebar, which the coordinator approved for this package. While deriving the session generator we found that `Pcg64Mcg::new` sets the two lowest state bits, so the `session_rng` example in [Coding Standards](Coding-Standards.md) gives one stream to four neighboring session ids, and we filed bug #90. The evidence is `cargo test -p fps-hosts` and `docs/handoff/HST-01.md` on the branch `wp/HST-01`.

### Shared page notes

- [Testing](Testing.md) could name `fps-hosts` in the `test-eng` row, which already runs it through the engine crate glob.
- [Coding Standards](Coding-Standards.md) should replace the `session_rng` example once bug #90 lands, because the current example collapses session ids that differ only in the two lowest bits.
- The example plan `contracts/examples/plans/http_dns_smoke.json` gives the pool `servers` a `poolCount` of 2, while `contracts/examples/tests/http_smoke.yaml` gives it a `count` of 1. A CON package may want to align them.
