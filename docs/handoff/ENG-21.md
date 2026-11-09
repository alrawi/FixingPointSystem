# Handoff ENG-21 Extension seams found by AGT-11

| Field | Value |
|---|---|
| Issue | #338 |
| Branch | `wp/ENG-21` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-e21a7c, lease until 2026-11-10T00:39:04Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Export `fps_runtime::close_reason`, add the transport to `SessionKey` so a UDP session's extension sees `udp`, and let a caller load an extension from the bytes it digested, so the tree digest and the compiled component describe the same bytes.

## Plan

- [ ] 1. Export `close_reason`, add `SessionKey::transport`, pass it to the extension
- [ ] 2. Add `ExtensionSet::load_module_bytes` (and keep `load_module_dir`), with tests
- [ ] 3. Update `docs/wiki/Engine.md`, the log entry, run checks, open the PR

## Done and verified

Nothing yet.

## Remaining

All steps.

## Next action

Edit `engine/fps-runtime/src/extension.rs`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-runtime --test extensions
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-e21a7c | claim | none | claimed #338 |
