# Handoff TRN-15 TLS plaintext backpressure and mail and h2 profiles

| Field | Value |
|---|---|
| Issue | #374 |
| Branch | `wp/TRN-15` |
| Pull request | none yet |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-t15b7e, lease until 2026-11-10T14:16:49Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`TlsSession::poll_fill` fails with "received plaintext buffer full" when one ciphertext chunk completes more than 16 KiB of plaintext while earlier plaintext is unread (seen in MOD-06). Treat the full buffer as backpressure: stop reading, keep the ciphertext, and resume after the application drains. Add a mail client profile without ALPN and the server profiles `server-tls13-h2` and `server-tls13-noalpn`. A TLS test moves a 1 MiB body in one burst both ways, and handshakes with the new profiles negotiate the expected ALPN.

## Plan

- [x] 1. Claim commit and handoff
- [x] 2. Backpressure in `TlsSession` (`native/src/session.rs`) with a failing test first (`tests/burst.rs`)
- [x] 3. Profiles `mail-client`, `server-tls13-h2`, `server-tls13-noalpn` (`native/src/mail.rs`) and their handshake tests
- [x] 4. Transport wiki page, module README, log entry
- [ ] 5. Checks, pull request, CI

## Done and verified

2: the burst tests run against the unchanged `session.rs` failed with `received plaintext buffer full` (3 of 8 failed), and all 8 pass with the change. 3: `cargo test -p fps-module-tls` passes (13 unit, 8 burst, 2 capture, 4 ja4, 14 sessions tests, tshark present). `cargo clippy -p fps-module-tls --all-targets -- -D warnings` and `cargo fmt -p fps-module-tls -- --check` pass. 4: `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Open the pull request and wait for CI. The workspace wide `make lint test` is left to CI because of the shared machine.

## Next action

Open the pull request and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- The coordinator fixed the two server profile names. The issue gives no name for the mail client, so it is `mail-client` (the Firefox 139 lists without ALPN, because Thunderbird shares the TLS library of Firefox). AGT-16 must use that name or ask for a rename.
- Backpressure is detected as `ErrorKind::Other` from `read_tls`, which rustls documents as its backpressure signal. The session keeps the refused ciphertext in its own read buffer (`in_pos..in_len`) and reads the transport only when that buffer is empty.
- The server profiles carry `ServerResumption::Disabled`, like `server-tls13` of the agent, so a shared connector never resumes. AGT-16 may set `Tickets` once it builds one connector per session.

## Pitfalls

- A single `write_all` of 1 MiB does not reproduce the bug: one record of 16 KiB plaintext per chunk stays under the limit. It needs a full record plus small records in one 16.5 KiB chunk, so `burst.rs` cuts the body in patterns of 16 KiB and 1 byte, and of 9 byte headers and 16 KiB payloads.
- The sandbox worktree guard refuses `git` combined with other commands in one shell call.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-TRN-15 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-module-tls
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T12:47Z | claude-t15b7e | claim | none | claimed issue #374, card 6767704760 |
| 2026-11-10T13:10Z | claude-t15b7e | checkpoint | none | backpressure, profiles, tests, wiki; ready to open the pull request |
