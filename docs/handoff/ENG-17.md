# Handoff ENG-17 Load WebAssembly extensions into sessions

| Field | Value |
|---|---|
| Issue | #231 |
| Branch | `wp/ENG-17` |
| Pull request | #284 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-e17a3b, lease until 2026-11-09T14:07:00Z |
| Last verified commit | b6dbd8c |

## Goal

The runtime compiles a module's extension.wasm once and instantiates it per session through fps-wasm, with a per-session Host whose randomness derives from the run seed and session id. A render failure carrying a WasmFailure maps to extension_error, and a looping extension fails only its own session.

## Plan

- [x] 1. Read ENG-11, ENG-15, ENG-19 code and the extension ABI
- [x] 2. FailureKind::reason maps WasmFailure to extension_error
- [x] 3. Runtime loads and compiles extension.wasm once; per-session Host
- [x] 4. WAT fixture tests including looping extension
- [x] 5. Wiki page, log entry, checks, PR

## Done and verified

- `engine/fps-runtime/src/extension.rs`: `ExtensionSet`, `SessionExtensions`, `SessionHost`, `extension_rng`. `FlowRef::extensions`, worker wiring in `worker.rs`.
- `cargo test -p fps-runtime --test extensions`: 7 passed, including the looping-extension acceptance test.
- `cargo clippy -p fps-runtime --all-targets -- -D warnings`: clean. `cargo fmt --all -- --check`: clean. `plan.py check`, `render --check`, `lint`: ok.
- FailureKind::reason needed no change (ENG-15 maps every TemplateError::Function to extension_error); verified end to end by the runtime test counting reason `extension_error`.

## Remaining

PR, CI, review. Follow ups are in the log entry.

## Next action

Read engine/fps-wasm and engine/fps-interp.

## Blockers and open questions

none

## Decisions taken in this package

- No change to fps-interp or fps-wasm: the reason mapping already exists and `WasmFunctions` is not used, because the runtime opens instances lazily and needs the module fallback.
- Instances open at the first template call of the module in a session, not after connect, because the interpreter has no hook seam.

## Pitfalls

The fps-wasm test component refuses session id 13 in on-open. Worker 0 local session 13 has id 13, so the runtime test uses worker ids 20 and 21.

## Verify the current state

```bash
cargo test -p fps-runtime -p fps-interp -p fps-wasm
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-e17a3b | claim | none | claimed #231 |
| 2026-11-09 | claude-e17a3b | finish | b6dbd8c | PR #284, CI green, ready for review |
