# Handoff RLM-12 A current nDPI and a shared decrypted view in the realism runner

| Field | Value |
|---|---|
| Issue | #373 |
| Branch | `wp/RLM-12` |
| Pull request | #384 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-7a1786, lease until 2026-11-10T14:27:28Z |
| Last verified commit | 0c3bcea |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The realism CI installs libndpi-bin 4.2.0, which has no HTTP/2. Build a pinned nDPI 4.14 in `realism/ci/install-tools.sh`, let the classifier read both CSV dialects (4.14 uses a pipe separator) and tolerate `ndpiReader -h` exiting 1, and add `python3 -m realism decrypted --pcap F --labels L` that replaces the `decrypted_check.py` copies of http1, smtp, imap, and http2.

## Plan

- [x] 1. Claim commit, handoff
- [x] 2. Classifier reads both CSV dialects and tolerates `-h` exit 1, with tests
- [x] 3. `realism decrypted` (view builder, scoring, CLI), with tests and example labels
- [x] 4. Pinned nDPI 4.14 build, HTTP/2 evidence test, docs, log entry, pull request, CI

## Done and verified

- `make lint-rlm test-rlm`: 492 passed, 5 skipped (Zeek tests, the http2 fixture test without a fixture in the tree). Run with the real tshark 4.2.2 and the real nDPI 4.14.0 that `build-ndpi.sh` built in the sandbox.
- `realism decrypted` on the committed fixtures: http1 80 of 80, smtp 8 of 8, imap 8 of 8, and on the http2 fixture of `wp/MOD-06` 40 of 40 TLS sessions decrypt and nDPI 4.14 names every view session `HTTP`, `SMTP`, `IMAP`, and `HTTP2`.
- nDPI 4.14 names 40 of 40 cleartext sessions of the http2 fixture `HTTP2`.

## CI

All checks of commit 0c3bcea passed. The `rlm` job built nDPI 4.14.0-1-90090b9, ran 497 tests with none skipped, and printed `nDPI 4.14.0-1-90090b9 classifies 40 of 40 cleartext sessions of the http2 fixture (server ports [8081, 8082]) as HTTP2, ratio 1.0000` and the decrypted counts 80 of 80, 8 of 8, 8 of 8, and 40 of 40. Every module of the `realism` workflow passed with nDPI 4.14.

## Remaining

Review by the coordinator and the switch of the module workflows (see the log entry).

## Next action

Coordinator review of pull request #384.

## Blockers and open questions

none

## Decisions taken in this package

- Scope is `realism/` only, as the coordinator asked. Module workflows and `modules/` stay untouched, and the log entry lists the exact commands for them.
- The pin is a tag and its commit id, fetched by commit, because a GitHub tag archive is not byte stable. The build runs in `build-ndpi.sh`, not in a cache step, because workflows are outside the package paths.
- The labels file describes the decrypted view. The example labels for the four fixtures live in `realism/examples/decrypted/`.
- The command scores nDPI conformance and the parser error check of the view. The per protocol counts of the old scripts and the SMTP message check of Zeek stay with the modules.

## Pitfalls

- tshark 4.2 misses the first TLS session on a port it does not know when the ClientHello spans two segments, so `--sessions` guards the count. Tests use port 587 for STARTTLS.
- A test that runs `build-ndpi.sh` with an empty `FPS_NDPI_VERSION` falls back to the real pin and builds nDPI. Never pass an empty pin.

## Verify the current state

```bash
make lint-rlm test-rlm PYTHON=python3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T12:50Z | claude-7a1786 | claim | none | claimed #373 |
| 2026-11-10T12:58Z | claude-7a1786 | checkpoint | 1f2416d | classifier dialects, decrypted command |
| 2026-11-10T13:20Z | claude-7a1786 | review | 0c3bcea | pull request #384, CI green |
