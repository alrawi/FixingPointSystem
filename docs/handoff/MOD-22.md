# Handoff MOD-22 Shared decrypted check in the module capture scripts

| Field | Value |
|---|---|
| Issue | #394 |
| Branch | `wp/MOD-22` |
| Pull request | #397 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-97754f, lease until 2026-11-10T15:53:19Z |
| Last verified commit | 2f40520 (CI: realism passes for the four modules) |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The four `capture.sh` scripts of http1, http2, smtp and imap call `python3 -m realism decrypted` instead of their own `decrypted_check.py` copies. The copies, their tests and `modules/http2/fixtures/ndpi_reader_compat.py` go. The realism workflow passes for the four modules, and no module file names `decrypted_check.py`.

## Plan

- [x] 1. Claim commit and handoff file.
- [x] 2. Switch the four `capture.sh` scripts and the Rust fixture tests to the shared command.
- [x] 3. Remove the copies, their tests and the compat wrapper; update READMEs, expected.yaml comments, log entry with the workflow and mk edits.
- [x] 4. Checks, PR, CI.

## Done and verified

- Four `capture.sh` call `python3 -m realism decrypted`; `bash -n` and `shellcheck -S warning` pass.
- Copies, tests, compat wrapper and the old `http1.decrypted.json` removed; `grep -rn decrypted_check modules` finds nothing.
- `tests/fixture.rs` of the four modules rewritten for the shared command, `rustfmt` clean (not compiled here).
- `python3 -m unittest discover -s modules/smtp/fixtures` OK; plan check, render --check, lint, `make lint-mod-*` OK.

## Remaining

The coordinator pushes the workflow edits of the log entry (`mod-imap.yml`, `mod-http2.yml`, `mod-http1.yml`). `mod-imap` (module) and `mod-http2` (module, harness) are red until then.

## Next action

Coordinator: push the workflow edits, then merge.

## Blockers and open questions

none

## Decisions taken in this package

- smtp `capture.sh` keeps the view (`--view`) and runs `zeek_check.py` on it, so the old Zeek message check of the view stays covered.
- Session counts come from the script's `sessions` variable times the https scenarios of the module.

## Pitfalls

- `qa-gate1` failed once on one http1 parser error of its live controller and agent run (1339 of 1340 sessions captured) and passed on rerun; it does not use the module scripts.

## Verify the current state

```bash
git log --oneline origin/main..wp/MOD-22
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T15:23Z | claude-97754f | claim | none | claimed issue 394 |
