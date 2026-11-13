# Handoff MOD-30 STARTTLS ports in the smtp and imap capture scripts

| Field | Value |
|---|---|
| Issue | #546 |
| Branch | `wp/MOD-30` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (3 of 3 plan steps, CI pending) |
| Current holder | claude-4d2f9a, lease until 2026-11-13T18:06:18Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

RLM-21 added `--starttls-port PORT:PROTOCOL` to `python3 -m realism decrypted`. Pass `--starttls-port 5587:smtp` and `--starttls-port 1143:imap` in `modules/smtp/fixtures/capture.sh`, `modules/imap/fixtures/capture.sh`, and the fixture tests `modules/smtp/tests/fixture.rs` and `modules/imap/tests/fixture.rs`, so they no longer rely on the tshark TLS heuristic.

## Plan

- [x] 1. Add the option to both capture scripts and both fixture tests.
- [x] 2. Score the committed captures locally with the option, where tshark and ndpiReader exist.
- [x] 3. Log entry, checks, PR (CI pending).

## Done and verified

Edits to both capture scripts, both fixture tests, both READMEs, Module-Development. Verified: `capture.sh` of each module in the namespace harness (4 sessions) passes with the option; `realism decrypted --starttls-port` on the committed captures passes 8 of 8; `cargo test -p fps-module-{smtp,imap} --test fixture` pass; plan.py check, render --check, lint, lint-mod-smtp, lint-mod-imap pass.

## Remaining

CI result, then `fps_swarm.py review`.

## Next action

Poll CI on the PR.

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
