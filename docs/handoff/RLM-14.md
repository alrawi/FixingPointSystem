# Handoff RLM-14 Message counts in the decrypted view

| Field | Value |
|---|---|
| Issue | #409 |
| Branch | `wp/RLM-14` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-790651, lease until 2026-11-11T00:05:08Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

MOD-22 replaced the module copies of `decrypted_check.py` with `python3 -m realism decrypted`, which does not count HTTP requests and responses inside TLS, the SMTP command and reply sequence, or the IMAP commands and literals that the old scripts checked. Add optional message count expectations to the decrypted labels, read with tshark from the view, set them in the four example labels with literal counts that the committed fixtures pass, and make a view with a missing message fail with a named reason.

## Plan

- [ ] 1. Claim commit, handoff
- [ ] 2. Labels section `messages` (parser, schema) and the `messages` check with a tshark probe and stub, with unit tests
- [ ] 3. Report schema and format 0.11.0, `realism decrypted` wiring, example labels with literal counts, fixture tests
- [ ] 4. README, Realism page, log entry, checks, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything after the claim.

## Next action

Write `realism/messages.py` and the labels section.

## Blockers and open questions

none

## Decisions taken in this package

Recorded at the end of the package.

## Pitfalls

- The sandbox has tshark 4.2.2 and nDPI 4.2.0 (no HTTP/2), so the http2 view fails the conformance check locally. CI builds nDPI 4.14.

## Verify the current state

```bash
python3 -m pytest realism -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T22:40Z | claude-790651 | claim | none | Claimed #409, started the branch from origin/main |
