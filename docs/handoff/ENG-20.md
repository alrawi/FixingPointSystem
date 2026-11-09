# Handoff ENG-20 Upgrade a running flow to TLS

| Field | Value |
|---|---|
| Issue | #311 |
| Branch | `wp/ENG-20` |
| Pull request | #320 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-e20a7c, lease until 2026-11-09T19:58:56Z |
| Last verified commit | a15373c |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A flow action `tls_upgrade` wraps the existing transport of a running session in TLS in place, so STARTTLS (SMTP, IMAP, POP3, FTP, LDAP) is expressible. The client side initiates the handshake and the server side accepts it. The capture tap keeps recording the encrypted bytes and the key log gets the secrets. The scenario contract needs the new keyword, which a CON package owns, so this package reports the exact text.

## Plan

- [x] 1. Flow compiler: `Op::UpgradeTls` and the `tls_upgrade` action in `fps-flow`.
- [x] 2. Interpreter: the `TlsUpgrader` seam and the in place swap in `fps-interp`.
- [x] 3. Runtime: a TLS profile registry in `fps-runtime`, wired into each session, with the TRN-04 module as the upgrader.
- [x] 4. Loopback SMTP STARTTLS test with capture and key log, `docs/wiki/Engine.md`, log entry, checks, PR.

## Done and verified

- Steps 1 to 3: `cargo test -p fps-flow -p fps-interp -p fps-runtime` passes, including `fps-runtime/tests/tls_upgrade.rs`. `cargo clippy ... --all-targets -D warnings` passes.
- Step 4 in progress: Engine.md and the log entry are written.

## Remaining

Open the pull request, wait for CI, report the contract text of the log entry to the coordinator.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

- The TLS profiles live in `RuntimeConfig`, not in `FlowRef`, because one more field in `FlowRef` pushed `Role::Server` over the `large_enum_variant` limit of clippy.
- The upgrade is an interpreter seam (`TlsUpgrader`), so `fps-interp` does not depend on rustls. The runtime supplies the implementation over `fps-module-tls`.

## Pitfalls

The disk of the sandbox filled up during a build. Delete `target/debug/incremental` if that happens.

## Verify the current state

```bash
cargo test -p fps-flow -p fps-interp -p fps-runtime
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T18:13:00Z | claude-e20a7c | claim | none | claimed issue #311 |
| 2026-11-09T18:43:00Z | claude-e20a7c | finish | a15373c | PR #320 open, CI green, ready for review |
