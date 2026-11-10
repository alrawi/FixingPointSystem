# Handoff MOD-22 Shared decrypted check in the module capture scripts

| Field | Value |
|---|---|
| Issue | #394 |
| Branch | `wp/MOD-22` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-97754f, lease until 2026-11-10T15:53:19Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The four `capture.sh` scripts of http1, http2, smtp and imap call `python3 -m realism decrypted` instead of their own `decrypted_check.py` copies. The copies, their tests and `modules/http2/fixtures/ndpi_reader_compat.py` go. The realism workflow passes for the four modules, and no module file names `decrypted_check.py`.

## Plan

- [ ] 1. Claim commit and handoff file.
- [ ] 2. Switch the four `capture.sh` scripts and the Rust fixture tests to the shared command.
- [ ] 3. Remove the copies, their tests and the compat wrapper; update READMEs, expected.yaml comments, log entry with the workflow and mk edits.
- [ ] 4. Checks, PR, CI.

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Edit `modules/http1/fixtures/capture.sh`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
git log --oneline origin/main..wp/MOD-22
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T15:23Z | claude-97754f | claim | none | claimed issue 394 |
