## [2026-11-06] ingest | ENG-11 WebAssembly extension host

We added the crate `engine/fps-wasm/`, which loads components of the `extension` world with wasmtime 48 and runs their hooks. `WasmFunctions` implements `fps_flow::TemplateFn`, so the renderer reaches WebAssembly and native extensions through one trait. Every hook call gets a fresh fuel budget, and every session instance has a memory limit that turns a refused growth into a trap. Every extension failure maps to the failure reason `extension_error` of `fps-stats`, with the finer label `extension_<kind>`. The acceptance test builds an extension from WAT text, shows that an endless loop stops at its fuel limit, and shows that a second session of the same extension keeps working. We added the section [The WebAssembly extension host](Engine.md#the-webassembly-extension-host) to the Engine page. The evidence is `docs/handoff/ENG-11.md`.

## Shared page notes

- [Engine](Engine.md), crate table. The `fps-wasm` row is already in the table, and the `fps-interp` and `fps-runtime` rows can say that they wire `WasmFunctions` into the session module table.
- Contract note for `contracts/extension/README.md`. It says that the engine instantiates the component once per worker. `fps-wasm` instantiates once per session, because a trap leaves an instance unusable, and one instance per worker would let a looping extension fail every session of its worker. The text can say "once per session from one compiled component".
- Contract note for the same page. The `random-u64` and `random-bytes` calls have no error case, so the table cell `not-allowed` for `on-close` can only be a trap. `fps-wasm` traps the guest.
- Follow up for `engine/fps-interp/`. `FailureKind::reason` can return `Reason::ExtensionError` when a render failure carries a `WasmFailure`. `fps_flow::FnErrorKind` has no case for traps and limits, so the adapter reports them as `Internal` and keeps the `WasmFailure` in `WasmFunctions::take_failure`.
- Follow up for the module loader. Nothing yet reads `extension.wasm` from the manifest and builds a `WasmExtension`, and nothing implements the `Host` trait for a session. The seeds for `random_u64` should derive from the run seed and the session id.
- Follow up for decision record 0010. Its conformance tests need a native extension and the `Extension` and `ExtensionSession` traits, which no crate defines yet.
- Root `Cargo.lock` gained the wasmtime tree (70 packages). Every license is on the `deny.toml` allow list, and `cargo deny` was not run locally.
