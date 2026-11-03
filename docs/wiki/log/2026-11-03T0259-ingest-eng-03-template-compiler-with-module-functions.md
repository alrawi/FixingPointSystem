## [2026-11-03] ingest | ENG-03 Template compiler with module functions

We replaced the v0.1 template parser in `engine/fps-flow` with a compiler for the full v0.2 grammar, which follows the reference parser in `tools/contracts/validate.py`. Module calls compile into call slots with keyword arguments and nested calls, and they dispatch at render time through the new trait `fps_flow::TemplateFn`, whose argument types mirror the extension interface. The compiler folds session independent parts into literal segments, and `Scenario::fold_constants` folds the run variables that no `set` binds. `Template::dump` and `Scenario::dump` give a stable program dump that the golden tests compare. All five example scenarios now compile, including `dns.resolve` and `http1.web_page`. We described the design in the Templates section of [Engine](Engine.md). The evidence is `docs/handoff/ENG-03.md`, the golden files under `engine/fps-flow/tests/testdata/`, and the property test that rendering is deterministic for a fixed seed and session id.

## Shared page notes

- [Testing](Testing.md), section Golden files. The example call `UPDATE_GOLDEN=1 cargo test -p fps-flow` now rewrites real files, the grammar golden `grammar.seed42.txt` and the example dumps under `tests/testdata/examples/`. No text change is required.
- [Home](Home.md). ENG-03 is ready for review, and ENG-04 can start from the `TemplateFn` seam and the new `last` and `functions` fields of `RenderCtx`.
