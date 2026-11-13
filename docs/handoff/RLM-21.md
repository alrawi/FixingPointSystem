# Handoff RLM-21 STARTTLS ports in the decrypted view

| Field | Value |
|---|---|
| Issue | #526 |
| Branch | `wp/RLM-21` |
| Pull request | #539 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-c41e7a, lease until 2026-11-13T05:43:08Z |
| Last verified commit | f6ec76d (30 of 30 check runs green) |

## Goal

The smtp and imap module jobs score their decrypted view with `--view-port`, so the labels name the view's port and the runner cannot know the capture's STARTTLS ports. A STARTTLS session starts in clear text, so decoding its port as TLS is wrong. Give the runner a way to name the capture ports of STARTTLS sessions that decodes TLS only after the upgrade, and use it in both jobs.

## Plan

- [x] 1. `--starttls-port PORT:PROTOCOL` decodes the port as the application protocol (`-d tcp.port==P,smtp`), whose tshark dissector switches to TLS after STARTTLS.
- [x] 2. Tests: split ClientHello STARTTLS captures (smtp 5587, imap 1143, cut 1 and 4) list every session; the smtp and imap workflows and fixture tests use the option.
- [x] 3. README, Realism page, log entry, checks, pull request, CI.

## Done and verified

- `python3 -m pytest realism/tests/test_decrypted.py`: 52 passed with real tshark 4.2.2 and openssl.
- `ruff format --check`, `ruff check`, `mypy` on `realism`: clean.

## Remaining

Nothing. The fixture tests of the two modules skip locally (nDPI 4.14 is missing); CI ran them (`module (smtp)`, `module (imap)` green).

## Next action

Coordinator: apply the Module Development text of the log entry, then merge.

## Blockers and open questions

none

## Decisions taken in this package

- Decode As with the application protocol, not TLS: tshark 4.2 lists 0 of 3 sessions for a split ClientHello and 2 of 3 without a split when the STARTTLS port is decoded as TLS, because the plain greeting is fed to the TLS dissector. Decoded as smtp, imap, or pop, it lists all three.
- An explicit option, because the labels of a view with `--view-port` name the port of the view. The report format is unchanged (0.17.0), no schema change.
- A port in both the option and the labels rules is decoded as the protocol.
- `modules/smtp/fixtures/capture.sh`, `modules/imap/fixtures/capture.sh` and the Rust fixture tests call `decrypted` without the option. That still works (heuristic); a coordinator follow up can add it.

## Pitfalls

`tlsgen.add_session(first_segment=...)` cut the first client flight, which in a STARTTLS dialogue is the EHLO. It now cuts the first client flight that starts with a handshake record.

## Verify the current state

```bash
python3 -m pytest realism/tests/test_decrypted.py -q   # 52 passed
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-13T04:13Z | claude-c41e7a | claim | none | claimed #526 |
| 2026-11-13T04:40Z | claude-c41e7a | finish | f6ec76d | PR #539, CI green |
