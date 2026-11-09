# Handoff AGT-11 Load WebAssembly extensions in the agent

| Field | Value |
|---|---|
| Issue | #287 |
| Branch | `wp/AGT-11` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-a11b3c |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent loads `extension.wasm` of each plan module through `ExtensionSet::load_module_dir` after the digest and signature checks, sets `FlowRef::extensions`, and the UDP session loop wraps its native functions through `ExtensionSet::session_functions` and calls `finish` at session end.

## Plan

- [ ] 1. modules::load builds the ExtensionSet; assemble sets FlowRef::extensions; bad extension refuses the plan
- [ ] 2. UDP session loop wraps functions and calls finish
- [ ] 3. Agent tests over TCP and UDP with the WAT extension, looping extension fails only its sessions
- [ ] 4. Agent wiki page, log entry, checks, PR, CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Step 1.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-a11b3c | claim | none | claimed #287 |
