# Handoff RLM-18 TLS on non standard ports in the decrypted check

| Field | Value |
|---|---|
| Issue | #495 |
| Branch | `wp/RLM-18` |
| Pull request | #516 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-71aa28, lease until 2026-11-12T17:55:25Z |
| Last verified commit | 712f31b (54 of 54 check runs green) |

## Goal

`realism/decrypted.py` lists ClientHellos with plain tshark, which finds TLS on the ports 8443 and 8444 only by its heuristic, so a ClientHello split after its first bytes is missed (MOD-23). `read_sessions` must decode the server ports of the labels as TLS. The http2 wrapper in `modules/http2/` stays, because MOD-24 removes it.

## Plan

- [x] 1. `Tshark.run` and `read_sessions` pass `-d tcp.port==P,tls` for the server ports of the labels.
- [x] 2. Tests: a split ClientHello capture on a non standard port lists every session, and the labels give the ports.
- [x] 3. Runner README and Realism page, log entry, checks, pull request, CI.

## Done and verified

- `python3 -m pytest realism -q`: 812 passed, 9 skipped, 1 failed. The failure is `test_pinned_ndpi.py::test_a_reader_of_another_release_is_not_accepted`, which builds nDPI and hit "No space left on device" (the sandbox disk has 106 MB free), so it is environmental.
- `ruff format --check`, `ruff check`, `mypy` on `realism`: clean. `plan.py check`, `render --check`, `lint`: ok.
- The real command on the committed fixtures: http1 80 of 80 TLS sessions, http2 40 of 40 (its conformance fails locally only because the sandbox nDPI is 4.2.0, which has no HTTP/2).

## Remaining

Nothing. The coordinator reviews and merges.

## Next action

Review pull request #516.

## Blockers and open questions

none

## Decisions taken in this package

With `--view-port` no port is added, because the labels then name the port of the view (smtp 587, imap 143), not of the capture. The report format stays 0.17.0 (no field changed). The http2 wrapper stays for MOD-24.

## Pitfalls

Do not use `git stash` here (shared stack). The sandbox disk is nearly full, which breaks the nDPI build test.

## Verify the current state

```bash
python3 -m pytest realism/tests/test_decrypted.py -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12 | claude-71aa28 | claim | none | claimed #495 |
| 2026-11-12 | claude-71aa28 | checkpoint | see branch | fix, tests, docs, log entry |
| 2026-11-12 | claude-71aa28 | finish | 712f31b | CI green, moved to review |
