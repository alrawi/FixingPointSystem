## [2026-11-03] ingest | AGT-01 Standalone agent mode

We added `fps-agent run --plan FILE --modules DIR` to `engine/fps-agent` and documented it on the new component page [Agent](Agent.md). The command reads a plan in the canonical JSON mapping, loads each module from `DIR/<name>` after checking its manifest version and its tree digest, checks and compiles every scenario source, registers the native functions of `http1` and `dns` when the manifest catalog equals the crate catalog, and runs the workers. TCP flows run on `fps-runtime`, and UDP flows run on agent worker threads with the datagram openers of `fps-backend-kernel`, because the runtime has no UDP worker yet. The process prints one aggregate `CounterSnapshot` per second as a JSON line, evaluates the plan criteria from the end of the warmup, and exits with 0 for pass, 1 for fail, and 2 for any error. The options `--side client` and `--side server` split the two sides across processes. The example plan `http_dns_smoke.json` ran unchanged in the QA-01 harness with 38,107 successful sessions of both modules on both arms. The example manifests list other function catalogs than the first party crates, so the example modules run without native functions, which their scenarios do not call. The crate now depends on the engine crates, the two native module crates, `serde`, `serde_json`, `hdrhistogram`, `rand`, `core_affinity`, `tokio`, and the new crate `sha2`, so `Cargo.lock` changed. The evidence is `docs/handoff/AGT-01.md` on the branch `wp/AGT-01`.

## Shared page notes

The coordinator may apply the notes below.

1. [Home](Home.md) and the sidebar may link the new page [Agent](Agent.md).
2. [Testing](Testing.md) may extend the `test-agt` row with the standalone mode component tests in `engine/fps-agent/tests/standalone.rs`, and add the row `e2e-agt`, which runs `engine/fps-agent/tests/netns_smoke.sh` as root in the QA-01 harness through the workflow `agt.yml`.
3. [Implementation Plan](Implementation-Plan.md) may mark AGT-01 as done after the merge, and may move the AGT-03 paragraph to [Agent](Agent.md).
4. [Engine](Engine.md) may note that `fps-runtime` runs TCP flows only and that the standalone mode runs UDP flows with its own workers in `engine/fps-agent/src/udp.rs`. A later ENG package may move that worker into the runtime, after which the agent drops it.
5. The example manifests in `contracts/examples/modules/` list older function catalogs than `modules/http1` and `modules/dns`. A CON package may align them, so the example plan runs with native functions.
