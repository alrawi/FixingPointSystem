# Handoff RLM-09 Transport behavior check

| Field | Value |
|---|---|
| Issue | #191 |
| Branch | `wp/RLM-09` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-35cacd, lease until 2026-11-10T15:52:54Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Compare the TCP handshake options of a capture with the selected OS profile (windows11, linux6, macos14 of TRN-03). Each OS profile fixture passes, and a fixture with a wrong TTL fails. Report format goes from 0.8.0 to 0.9.0.

## Plan

- [ ] 1. Claim commit, handoff
- [ ] 2. `transport` labels section, handshake reader, `realism/transport.py`, report and CLI wiring, schema, version 0.9.0
- [ ] 3. Tests on pcapgen captures and on the committed module fixtures
- [ ] 4. README, Realism page, log entry, checks, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything after step 1.

## Next action

Write `realism/transport.py` and the `transport` labels section.

## Blockers and open questions

none

## Decisions taken in this package

- Scope is `realism/` only. The profile files stay in `engine/fps-backend-kernel/profiles/` and the check reads their `reference` section, so there is one source of truth.

## Pitfalls

- The sandbox has no tshark, Zeek, or ndpiReader. The check reads TCP headers itself, so its tests need no tool.

## Verify the current state

```bash
make lint-rlm test-rlm
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T14:23Z | claude-35cacd | claim | none | Claimed #191 |
