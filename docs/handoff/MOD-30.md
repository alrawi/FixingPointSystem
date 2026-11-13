# Handoff MOD-30 STARTTLS ports in the smtp and imap capture scripts

| Field | Value |
|---|---|
| Issue | #546 |
| Branch | `wp/MOD-30` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-4d2f9a, lease until 2026-11-13T18:06:18Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

RLM-21 added `--starttls-port PORT:PROTOCOL` to `python3 -m realism decrypted`. Pass `--starttls-port 5587:smtp` and `--starttls-port 1143:imap` in `modules/smtp/fixtures/capture.sh`, `modules/imap/fixtures/capture.sh`, and the fixture tests `modules/smtp/tests/fixture.rs` and `modules/imap/tests/fixture.rs`, so they no longer rely on the tshark TLS heuristic.

## Plan

- [ ] 1. Add the option to both capture scripts and both fixture tests.
- [ ] 2. Score the committed captures locally with the option, where tshark and ndpiReader exist.
- [ ] 3. Log entry, checks, PR, CI.

## Done and verified

Nothing yet.

## Remaining

All steps.

## Next action

Edit the four files.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
git log --oneline -3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-13 | claude-4d2f9a | claim | none | claimed #546 |
