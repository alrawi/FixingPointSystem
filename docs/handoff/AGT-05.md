# Handoff AGT-05 Capture and rate update handlers

| Field | Value |
|---|---|
| Issue | #162 |
| Branch | `wp/AGT-05` |
| Pull request | #259 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-a05c1d, lease until 2026-11-09T04:21:27Z |
| Last verified commit | 1b8771d |

## Goal

Implement the CON-05 v0.2 commands UpdateRates, StartCapture, and StopCapture in the agent session (`engine/fps-agent/`). Acceptance: a fake controller test changes rates and downloads (or locates and reads) a capture.

## Plan

- [x] 1. Rate handles: entry names in assemble.rs, UDP handles, UpdateRates handler
- [x] 2. Capture cell, UDP tap, StartCapture and StopCapture handlers
- [x] 3. Fake controller tests (`tests/run_control.rs`), clippy, fmt
- [x] 4. Agent.md, log entry, PR #259, CI green

## Done and verified

- `cargo test -p fps-agent` passes, 31 unit tests and all component tests, including `tests/run_control.rs`.
- `cargo clippy -p fps-agent --all-targets -- -D warnings` and `cargo fmt -p fps-agent --check` pass.
- `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Nothing. CI is green on 1b8771d, including controller-e2e. One Go test (pki TestTokens) failed once and passed on rerun, unrelated to this package.

## Next action

Review PR #259.

## Blockers and open questions

none

## Decisions taken in this package

- UDP entries are driven: the UDP workers register their schedule handles in `UdpRuntime::rate_control()`, and `control::Rates` joins that registry with the TCP one.
- The digest covers one pcapng file made by concatenating the ring files oldest first.
- CON-05 defines no transfer, so the file stays on the agent at `control::capture_path(dir, plan_id, capture_id)`.
- Filters, named ports, and snap lengths are refused instead of ignored.
- TCP sessions are not captured because `fps-runtime` has no tap hook (a gap for an ENG follow up).
- Capture file errors use `ERROR_CODE_RESOURCE_EXHAUSTED`, because `INTERNAL` moves the agent to `FAILED`.

## Pitfalls

The sandbox worktree guard refuses compound git commands and long heredocs. Run git alone per call and write scripts with the editor.

## Verify the current state

```bash
CARGO_BUILD_JOBS=1 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08 | claude-a05c1d | claim | | |
| 2026-11-08 | claude-a05c1d | checkpoint | f41f8df | handlers and tests done |
| 2026-11-08 | claude-a05c1d | ready for review | 1b8771d | PR 259, CI green |
