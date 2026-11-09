# Handoff ENG-17 Load WebAssembly extensions into sessions

| Field | Value |
|---|---|
| Issue | #231 |
| Branch | `wp/ENG-17` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-e17a3b, lease until 2026-11-09T14:07:00Z |
| Last verified commit | none |

## Goal

The runtime compiles a module's extension.wasm once and instantiates it per session through fps-wasm, with a per-session Host whose randomness derives from the run seed and session id. A render failure carrying a WasmFailure maps to extension_error, and a looping extension fails only its own session.

## Plan

- [ ] 1. Read ENG-11, ENG-15, ENG-19 code and the extension ABI
- [ ] 2. FailureKind::reason maps WasmFailure to extension_error
- [ ] 3. Runtime loads and compiles extension.wasm once; per-session Host
- [ ] 4. WAT fixture tests including looping extension
- [ ] 5. Wiki page, log entry, checks, PR

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read engine/fps-wasm and engine/fps-interp.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-runtime -p fps-interp -p fps-wasm
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-e17a3b | claim | none | claimed #231 |
