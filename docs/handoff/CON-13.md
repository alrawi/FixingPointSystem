# Handoff CON-13 Capture download over the agent connection

| Field | Value |
|---|---|
| Issue | #265 |
| Branch | `wp/CON-13` |
| Pull request | #270 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-c13a7e, lease until 2026-11-09T05:23:53Z |
| Last verified commit | none |

## Goal

Let the controller fetch a finished capture by id, from an offset, in ordered chunks with the total size and the `StopCaptureResult` digest, over the agent initiated connection. Additive, with `contracts/changes/CON-13.md`. The Rust agent answers UNIMPLEMENTED until AGT-09.

## Plan

- [x] 1. Proto: `FetchCapture` command and `UploadCapture` RPC, version 0.2.2, change entry
- [x] 2. Rust mirrors, round trip test, UNIMPLEMENTED answer, contract test tool update, wiki
- [x] 3. PR and CI

## Done and verified

`buf format`, `buf lint`, and `buf breaking` against origin/main pass. `cargo test -p fps-agent` contract, detect, session, and run_control pass. `pytest tools/contracts` passes. CI at 5e9c676 is green except `TestGeneratedCodeIsCurrent`, which waits for regen-agentpb.

## Remaining

The regeneration of `controller/internal/agents/agentpb` by the coordinator.

## Next action

Coordinator runs regen-agentpb on this branch, then reviews PR #270.

## Blockers and open questions

Go bindings need the regen-agentpb workflow, which the coordinator runs.

## Decisions taken in this package

Upload RPC on the same connection instead of chunks on `Session`: a separate HTTP/2 stream gives its own flow control, so no credit scheme and no heartbeat starvation. Version 0.2.2 and not 0.3.0, because the controller admits agents by major.minor. The Rust agent reports 0.2.2 because the contract test ties `ENGINE_API` to the proto header, as for 0.2.1.

## Pitfalls

The sandbox worktree guard refuses commands that mention git refs in `buf breaking`; compare against a copy of the base proto in the scratchpad.

## Verify the current state

```bash
buf lint contracts/proto
cargo test -p fps-agent --test contract
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08 | claude-c13a7e | claim | none | start |
| 2026-11-08 | claude-c13a7e | review | 5e9c676 | PR #270 opened, regeneration pending |
