# Handoff CON-24 Capture drop counts in the agent protocol and the REST API

| Field | Value |
|---|---|
| Issue | #442 |
| Branch | `wp/CON-24` |
| Pull request | #0 |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-e327f4, lease per the status card on #442 |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`StopCaptureResult.dropped` (proto) and `CaptureFile.dropped` (REST) count sessions, not packets, and mix three causes. Add optional fields for sessions dropped by overflow, sessions dropped by size, and sessions cut at the stop, correct the text of `dropped` (sessions, still the sum), bump the versions, and list the consumers. The change is additive and the owner approved it on 2026-11-10.

## Plan

- [x] 1. Read the ENG-27 log entry, the proto, and the OpenAPI schema, and fix the design.
- [x] 2. Edit the proto, the OpenAPI document, the versions, and `docs/wiki/Contracts.md`.
- [x] 3. Write `contracts/changes/CON-24.md` and the log entry with the follow up list.
- [ ] 4. Run the checks, open the PR, wait for CI, and request review.

## Done and verified

- `python3 tools/contracts/validate.py`: 0 errors. `check_plans.py`: 0 errors. `pytest -q tools/contracts`: 290 passed, 1 skipped.
- The proto file compiles with `grpc_tools.protoc`.

## Remaining

`buf` and `oasdiff` are not installed in the sandbox, so the proto and openapi breaking checks run in CI only. The coordinator applies the follow up of the log entry (Rust constants, agent split, generated Go, controller pass through, UI schema, SDK).

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

- The three new proto fields are `optional`, so absent (agent of 0.2.3 or earlier) differs from 0. The cost is `*uint64` in Go and `Option<u64>` in Rust.
- Protocol takes a patch number (0.2.4) and REST a minor number (0.11.0), as CON-11, CON-13, and CON-15 did.
- `Capture` gets no property, because it carries no drop count. The split lives on `CaptureFile`.

## Pitfalls

`engine/fps-agent/tests/contract.rs` ties `ENGINE_API` to the proto header, so the agent job is red until the constant moves to 0.2.4.

## Verify the current state

```bash
python3 tools/contracts/validate.py   # contracts: 0 error(s)
python3 -m pytest -q tools/contracts  # 290 passed, 1 skipped
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10 | claude-e327f4 | claim and work | none | Claimed #442, edited the contract, wrote the entry and the log |
