# Handoff ENG-21 Extension seams found by AGT-11

| Field | Value |
|---|---|
| Issue | #338 |
| Branch | `wp/ENG-21` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (3 of 3 plan steps, PR and CI pending) |
| Current holder | claude-e21a7c, lease until 2026-11-10T00:39:04Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Export `fps_runtime::close_reason`, add the transport to `SessionKey` so a UDP session's extension sees `udp`, and let a caller load an extension from the bytes it digested, so the tree digest and the compiled component describe the same bytes.

## Plan

- [x] 1. Export `close_reason`, add `SessionKey::transport`, pass it to the extension
- [x] 2. Add `ExtensionSet::load_module_bytes` (and keep `load_module_dir`), with tests
- [x] 3. Update `docs/wiki/Engine.md`, the log entry, run checks, open the PR

## Done and verified

- `cargo test -p fps-runtime --test extensions`: 10 passed, including three new tests.
- `cargo clippy -p fps-runtime --all-targets -- -D warnings` and `cargo fmt --check`: clean.
- `plan.py check`, `render --check`, `lint`: ok.

## Remaining

Open the PR and wait for CI. The agent side changes are in the log entry and belong to a later AGT package.

## Next action

Open the PR and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- `Transport` is re-exported from `fps-wasm` and not redefined, so the value reaches the guest without a mapping.
- `load_module_bytes` is a new method, and `load_module_dir` calls it, so no caller breaks.
- `SessionKey` is a public struct, so adding the field breaks literals outside the crate. Only the UDP loop of `fps-agent` has one, and AGT-13 owns that file.

## Pitfalls

The test component of `fps-wasm` ignores the transport, so the test patches its `on-open` WAT text. The test asserts the patch applied.

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-runtime --test extensions
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-e21a7c | claim | none | claimed #338 |
| 2026-11-09 | claude-e21a7c | checkpoint | pending | code, tests, wiki, log entry |
