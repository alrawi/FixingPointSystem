# Handoff ENG-11 WebAssembly extension host

| Field | Value |
|---|---|
| Issue | #156 |
| Branch | `wp/ENG-11` |
| Pull request | #226 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-e11a7c, lease until 2026-11-06T22:35:15Z |
| Last verified commit | 7272a00 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Host module extensions that implement the extension world (`contracts/extension/fps-extension.wit`) with per call fuel limits and per instance memory limits. A WebAssembly template function plugs in behind `fps_flow::TemplateFn`, the trait that native extensions use today. A looping extension is stopped by its fuel limit and fails only its own session.

## Plan

- [x] 1. Crate `engine/fps-wasm` with wasmtime, the component bindings, and the limits
- [x] 2. The `TemplateFn` adapter and the hook API with phase rules
- [x] 3. Tests with WAT components, including the endless loop and the memory limit
- [x] 4. Engine wiki page, log entry, checks, pull request

## Done and verified

- Crate `engine/fps-wasm` with `WasmEngine`, `WasmExtension`, `WasmSession`, `WasmFunctions` (a `fps_flow::TemplateFn`), the `Host` trait, `Limits`, and `WasmFailure`. `cargo test -p fps-wasm` passes 15 tests and `cargo clippy -p fps-wasm --all-targets -- -D warnings` is clean.
- Acceptance: an extension built from WAT text that loops forever stops with `OutOfFuel`, and a second session of the same extension keeps working (`an_endless_loop_is_stopped_by_fuel_and_fails_only_its_session`).
- Licenses of the 70 new lock packages are all on the `deny.toml` allow list, checked from `cargo metadata`. `cargo deny` itself is not installed here.

## Remaining

Review.

## Next action

Review and merge PR #226.

## Blockers and open questions

none

## Decisions taken in this package

- One instance per session, not per worker, because a trap leaves an instance unusable. The Engine page and the log entry carry a note for the extension contract README.
- `Host` is owned by the session as a `Box<dyn Host>`, so the guest needs no borrowed pointer. The `rng` of `TemplateFn::render` is ignored, and randomness comes from the host.
- A refused `memory.grow` or table growth traps (`OutOfMemory`), where plain `StoreLimits` return -1.
- `random-*` during `on-close` traps, because the WIT signature has no error case.
- The WIP snapshot on the branch held only a `Cargo.lock` change and no crate. The lock was regenerated from the final `Cargo.toml`.

## Pitfalls

- Hand written component WAT fails validation on exported resource methods (`resource used in function does not have a name in this context`). The tests write core WAT and encode it with `wit-component` and the contract WIT file instead.
- A WAT component that imports an instance must export the types of that instance too, or validation says `instance not valid to be used as import`.
- `wasmtime` without the `threads` feature has no `Config::wasm_threads`.

## Verify the current state

```bash
CARGO_BUILD_JOBS=1 cargo test -p fps-wasm
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06 | claude-e11a7c | claim | none | Claimed #156 |
| 2026-11-06 | claude-afe977 | takeover | 97a492b | Resumed from the WIP snapshot, built the crate, tests, and wiki |
