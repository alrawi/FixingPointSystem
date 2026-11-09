# Handoff AGT-11 Load WebAssembly extensions in the agent

| Field | Value |
|---|---|
| Issue | #287 |
| Branch | `wp/AGT-11` |
| Pull request | - The fps-wasm test component refuses session id 13 in `on-open`, which is engine worker 0 local session 13, so the test plan uses plan workers 1 and 2.
- Builds: first cold build of fps-agent takes about 5 minutes with one job. |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-a11b3c |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent loads `extension.wasm` of each plan module through `ExtensionSet::load_module_dir` after the digest and signature checks, sets `FlowRef::extensions`, and the UDP session loop wraps its native functions through `ExtensionSet::session_functions` and calls `finish` at session end.

## Plan

- [x] 1. modules::load builds the ExtensionSet; assemble sets FlowRef::extensions; bad extension refuses the plan
- [x] 2. UDP session loop wraps functions and calls finish
- [x] 3. Agent tests over TCP and UDP with the WAT extension, looping extension fails only its sessions
- [x] 4. Agent wiki page, log entry, checks, PR, CI

## Done and verified

- Steps 1 to 3: `CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-agent` passes (all suites, including the new `tests/extensions.rs` with 5 tests), `cargo clippy -p fps-agent --all-targets -- -D warnings` is clean, `cargo fmt` applied.
- The acceptance test `a_looping_extension_fails_only_its_own_sessions_over_tcp_and_udp` runs a plan over loopback with TCP and UDP scenarios of the WAT extension: good sessions succeed on both sides, looping sessions fail with `extension_error` and no other reason.

## Remaining

Nothing.

## Next action

Coordinator review and merge.

## Blockers and open questions

none

## Decisions taken in this package

- Extension limits are `Limits::default()`; no plan setting exists.
- Missing `extension.wasm` is detected with one `exists()` call, so the engine starts only at the first extension.
- The UDP worker repeats `close_reason` from fps-runtime, which is `pub(crate)` (finding for ENG).
- A module with an extension drops the "no native functions" note, replaced by "runs its WebAssembly extension".

## Pitfalls

- The fps-wasm test component refuses session id 13 in `on-open`, which is engine worker 0 local session 13, so the test plan uses plan workers 1 and 2.
- Builds: first cold build of fps-agent takes about 5 minutes with one job.

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-a11b3c | claim | none | claimed #287 |
| 2026-11-09 | claude-a11b3c | finish | 44c7ae8 | PR #331, CI green |
