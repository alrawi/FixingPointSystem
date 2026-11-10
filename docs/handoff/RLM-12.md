# Handoff RLM-12 A current nDPI and a shared decrypted view in the realism runner

| Field | Value |
|---|---|
| Issue | #373 |
| Branch | `wp/RLM-12` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-7a1786, lease until 2026-11-10T14:17:15Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The realism CI installs libndpi-bin 4.2.0, which has no HTTP/2. Build a pinned nDPI 4.14 in `realism/ci/install-tools.sh`, let the classifier read both CSV dialects (4.14 uses a pipe separator) and tolerate `ndpiReader -h` exiting 1, and add `python3 -m realism decrypted --pcap F --labels L` that replaces the `decrypted_check.py` copies of http1, smtp, imap, and http2.

## Plan

- [ ] 1. Claim commit, handoff
- [ ] 2. Classifier reads both CSV dialects and tolerates `-h` exit 1, with tests
- [ ] 3. `realism decrypted` (view builder, scoring, CLI), with tests
- [ ] 4. Pinned nDPI 4.14 build in `install-tools.sh`, HTTP/2 evidence test, docs, log entry, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything after the claim.

## Next action

Step 2.

## Blockers and open questions

none

## Decisions taken in this package

- Scope is `realism/` only, as the coordinator asked. Module workflows and `modules/` stay untouched.

## Pitfalls

- The sandbox has no network route to the nDPI release host except git, no libpcap headers, no ndpiReader. The 4.14 build is proven by CI only.

## Verify the current state

```bash
python3 -m pytest -q realism
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T12:50Z | claude-7a1786 | claim | none | claimed #373 |
