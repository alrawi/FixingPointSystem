# Handoff RLM-14 Message counts in the decrypted view

| Field | Value |
|---|---|
| Issue | #409 |
| Branch | `wp/RLM-14` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-790651, lease until 2026-11-11T00:05:08Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

MOD-22 replaced the module copies of `decrypted_check.py` with `python3 -m realism decrypted`, which does not count HTTP requests and responses inside TLS, the SMTP command and reply sequence, or the IMAP commands and literals that the old scripts checked. Add optional message count expectations to the decrypted labels, read with tshark from the view, set them in the four example labels with literal counts that the committed fixtures pass, and make a view with a missing message fail with a named reason.

## Plan

- [x] 1. Claim commit, handoff
- [x] 2. Labels section `messages` (parser, schema) and the `messages` check with a tshark probe and stub, with unit tests
- [x] 3. Report schema and format 0.11.0, `realism decrypted` wiring, example labels with literal counts, fixture tests
- [ ] 4. README, Realism page, log entry, checks, pull request, CI

## Done and verified

- `python3 -m ruff format --check realism`, `ruff check realism`, `mypy --strict` (the realism config): clean.
- `python3 -m pytest -q realism`: 628 passed, 9 skipped (Zeek and nDPI 4.14 tests) with tshark 4.2.2 and nDPI 4.2.0.
- `python3 -m realism decrypted` on the four committed fixtures: the `messages` check passes for http1 (80 sessions), http2 (40), smtp (8), and imap (8). The http2 command fails locally only in conformance, because nDPI 4.2 has no HTTP/2 (CI has 4.14).
- `tests/test_messages.py` cuts one session of each fixture in half and requires a `missing` reason.
- `python3 tools/plan/plan.py check`, `render --check`, `lint`: ok.

## Remaining

Pull request and CI.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

- Counts are per session, with rules that select sessions by match fields, because the fixtures have several session shapes (the http1 scenarios send one or four requests by port) and a total hides one session that lost a message.
- The check is a report check (`messages`, report 0.11.0) and runs from the labels, so `score` and `decrypted` both get it. The labels stay at `version: 1`, as for the earlier optional sections.
- A message is the tshark field `imf.from` (RFC 5322 requires From), not `imf.message_id`.
- A rule that selects no session fails, and a missing tshark fails.

## Pitfalls

- The sandbox has tshark 4.2.2 and nDPI 4.2.0 (no HTTP/2), so the http2 view fails the conformance check locally. CI builds nDPI 4.14.

## Verify the current state

```bash
python3 -m pytest realism -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T22:40Z | claude-790651 | claim | a38b08d | Claimed #409, started the branch from origin/main |
| 2026-11-10T23:20Z | claude-790651 | checkpoint | see PR | Check, labels, schemas, tests, docs, log entry |
