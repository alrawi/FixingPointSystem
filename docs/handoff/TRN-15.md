# Handoff TRN-15 TLS plaintext backpressure and mail and h2 profiles

| Field | Value |
|---|---|
| Issue | #374 |
| Branch | `wp/TRN-15` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-t15b7e, lease until 2026-11-10T14:16:49Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`TlsSession::poll_fill` fails with "received plaintext buffer full" when one ciphertext chunk completes more than 16 KiB of plaintext while earlier plaintext is unread (seen in MOD-06). Treat the full buffer as backpressure: stop reading, keep the ciphertext, and resume after the application drains. Add a mail client profile without ALPN and the server profiles `server-tls13-h2` and `server-tls13-noalpn`. A TLS test moves a 1 MiB body in one burst both ways, and handshakes with the new profiles negotiate the expected ALPN.

## Plan

- [ ] 1. Claim commit and handoff
- [ ] 2. Backpressure in `TlsSession` (`native/src/session.rs`) with a failing test first (`tests/burst.rs`)
- [ ] 3. Profiles `mail-client`, `server-tls13-h2`, `server-tls13-noalpn` (`native/src/mail.rs`) and their handshake tests
- [ ] 4. Transport wiki page, module README, log entry, checks, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything after step 1.

## Next action

Write the burst test, run it against the unchanged `session.rs` to see it fail, then apply the fix.

## Blockers and open questions

none

## Decisions taken in this package

- The coordinator fixed the two server profile names. The issue gives no name for the mail client, so it is `mail-client` (the Firefox 139 lists without ALPN, because Thunderbird shares the TLS library of Firefox). AGT-16 must use that name or ask for a rename.

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-TRN-15 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-module-tls
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T12:47Z | claude-t15b7e | claim | none | claimed issue #374, card 6767704760 |
