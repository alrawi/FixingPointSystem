# Handoff ENG-11 WebAssembly extension host

| Field | Value |
|---|---|
| Issue | #156 |
| Branch | `wp/ENG-11` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-e11a7c, lease until 2026-11-06T22:35:15Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Host module extensions that implement the extension world (`contracts/extension/fps-extension.wit`) with per call fuel limits and per instance memory limits. A WebAssembly template function plugs in behind `fps_flow::TemplateFn`, the trait that native extensions use today. A looping extension is stopped by its fuel limit and fails only its own session.

## Plan

- [ ] 1. Crate `engine/fps-wasm` with wasmtime, the component bindings, and the limits
- [ ] 2. The `TemplateFn` adapter and the hook API with phase rules
- [ ] 3. Tests with WAT components, including the endless loop and the memory limit
- [ ] 4. Engine wiki page, log entry, checks, pull request

## Done and verified

none yet

## Remaining

Everything.

## Next action

Create the crate and build it with `CARGO_BUILD_JOBS=1 cargo build -p fps-wasm`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_BUILD_JOBS=1 cargo test -p fps-wasm
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06 | claude-e11a7c | claim | none | Claimed #156 |
