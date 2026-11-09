# Handoff ENG-20 Upgrade a running flow to TLS

| Field | Value |
|---|---|
| Issue | #311 |
| Branch | `wp/ENG-20` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-e20a7c, lease until 2026-11-09T18:53:00Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A flow action `tls_upgrade` wraps the existing transport of a running session in TLS in place, so STARTTLS (SMTP, IMAP, POP3, FTP, LDAP) is expressible. The client side initiates the handshake and the server side accepts it. The capture tap keeps recording the encrypted bytes and the key log gets the secrets. The scenario contract needs the new keyword, which a CON package owns, so this package reports the exact text.

## Plan

- [ ] 1. Flow compiler: `Op::UpgradeTls` and the `tls_upgrade` action in `fps-flow`.
- [ ] 2. Interpreter: the `TlsUpgrader` seam and the in place swap in `fps-interp`.
- [ ] 3. Runtime: a TLS profile registry in `fps-runtime`, wired into each session, with the TRN-04 module as the upgrader.
- [ ] 4. Loopback SMTP STARTTLS test with capture and key log, `docs/wiki/Engine.md`, log entry, checks, PR.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Add `Op::UpgradeTls` to `engine/fps-flow/src/program.rs`.

## Blockers and open questions

none

## Decisions taken in this package

- The upgrade is an interpreter seam (`TlsUpgrader`), so `fps-interp` does not depend on rustls. The runtime supplies the implementation over `fps-module-tls`.

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-flow -p fps-interp -p fps-runtime
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T18:13:00Z | claude-e20a7c | claim | none | claimed issue #311 |
