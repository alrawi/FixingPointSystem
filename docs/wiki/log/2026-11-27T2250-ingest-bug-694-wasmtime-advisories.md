## [2026-11-27] ingest | bug #694: wasmtime 48.0.9 clears new RustSec advisories

- `cargo deny check advisories` (CI job `licenses`) started failing on every pull request: RUSTSEC-2026-0566 through RUSTSEC-2026-0573 were published against wasmtime 48.0.8.
- `Cargo.lock` now pins wasmtime 48.0.9 and its matching cranelift, pulley and wasmtime-internal crates (`cargo update -p wasmtime --precise 48.0.9`). No manifest change: `engine/fps-wasm` keeps `wasmtime = "48"`.
- Module authors: no change to the module interface or to how modules are written.

### Shared page notes

None.
