# Handoff RLM-09 Transport behavior check

| Field | Value |
|---|---|
| Issue | #191 |
| Branch | `wp/RLM-09` |
| Pull request | #399 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-35cacd, lease until 2026-11-10T15:52:54Z |
| Last verified commit | c01f9e4 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Compare the TCP handshake options of a capture with the selected OS profile (windows11, linux6, macos14 of TRN-03). Each OS profile fixture passes, and a fixture with a wrong TTL fails. Report format goes from 0.8.0 to 0.9.0.

## Plan

- [x] 1. Claim commit, handoff
- [x] 2. `transport` labels section, handshake reader, `realism/transport.py`, report and CLI wiring, schema, version 0.9.0
- [x] 3. Tests on pcapgen captures and on the committed module fixtures
- [x] 4. README, Realism page, log entry, checks, pull request, CI

## Done and verified

- `make lint-rlm` and `python3 -m pytest -q realism`: 547 passed, 9 skipped (tool tests, no tshark, Zeek, or ndpiReader in the sandbox).
- `plan.py check`, `render --check`, and `lint` pass.
- Every OS profile capture passes, a SYN with TTL 64 against windows11 fails, and the committed fixtures match linux6 except the window scale (10, profile 7).

## CI

All checks of commit c01f9e4 passed. The `rlm` job ran 556 tests with none skipped, the `realism` workflow and every module workflow passed, and so did `ci`.

## Remaining

Review by the coordinator.

## Next action

Coordinator review of pull request #399.

## Blockers and open questions

none

## Decisions taken in this package

- Scope is `realism/` only. The profile files stay in `engine/fps-backend-kernel/profiles/` and the check reads their `socket` and `reference` sections, so there is one source of truth.
- The option order and the SYN window decide only in strict mode, because the kernel backend cannot set them (Transport page, limits). They show up in `deviations` either way.
- The TTL comparison is exact and the labels give the hops (`client_hops`, `server_hops`), so a TTL that is one too high fails.
- The report check name is `transport_behavior`; `metrics` is empty because the catalog has no metric for it.

## Pitfalls

- The sandbox has no tshark, Zeek, or ndpiReader. The check reads TCP headers itself, so its tests need no tool.
- The harness fixtures show window scale 10, not the 7 of linux6, so the example labels use `expect`.
- The worktree guard refuses commands that mix heredocs or `source` with other commands. Write scripts with the editor and run them alone.

## Verify the current state

```bash
make lint-rlm test-rlm
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T14:23Z | claude-35cacd | claim | 37cb700 | Claimed #191 |
| 2026-11-10T14:50Z | claude-35cacd | checkpoint | c01f9e4 | Check, schema, tests, docs written, pull request opened |
| 2026-11-10T15:00Z | claude-35cacd | finish | c01f9e4 | CI green, ready for review |
