# Handoff MOD-04 HTTPS scenarios

| Field | Value |
|---|---|
| Issue | #171 |
| Branch | `wp/MOD-04` |
| Pull request | none yet |
| Status | in progress |
| Progress | 40% (2 of 5 plan steps) |
| Current holder | claude-0c91ff, lease until 2026-11-10T00:38:53Z |
| Last verified commit | none |

## Goal

Add http1 scenarios over TLS 1.2 and TLS 1.3 that bind the tls transport with browser profiles (at least two), with ALPN http/1.1, an SNI, and certificates from the test PKI of the TLS module. Record the fixture with exported keys. The realism labels give each scenario's `tls_profile` and expected JA4, so the RLM-04 encryption check passes. nDPI must classify at least 99% of the decrypted fixture sessions as HTTP over TLS.

## Plan

- [x] 1. Four https scenarios, TLS profile registry for the tests, in memory tests with JA4 check
- [x] 2. Golden transcripts of the plaintext
- [ ] 3. fixture_peer over TLS, capture.sh records HTTP and HTTPS in one capture with the key log injected, expected.yaml with the `tls` section
- [ ] 4. Measure the decrypted fixture (tshark with the key log, nDPI on the decrypted view), README, Modules.md, log entry
- [ ] 5. Checks, pull request, CI

## Done and verified

- Scenarios `https_fetch` (chrome-136, TLS 1.3, port 8443), `https_keepalive_browse` (firefox-139, TLS 1.3, 8444), `https_fetch_tls12` (safari-18.5, TLS 1.2, 8445), `https_keepalive_browse_tls12` (edge-134, TLS 1.2, 8446).
- `cargo test -p fps-module-http1`: all tests pass, including `tests/https.rs` (negotiated version, ALPN, SNI, wire encryption, key log, JA4 equals `expected_ja4` of the profile).

## Remaining

Steps 3 to 5.

## Next action

Extend `native/examples/fixture_peer.rs` with `--ca` and `--keylog`, using `tests/common/tls.rs`.

## Blockers and open questions

none

## Decisions taken in this package

- The scenarios use `tls_upgrade` right after `connect` and `accept`. The flow level `transport: {module: tls}` of the schema is rejected by `Scenario::load` (`unsupported a transport module`), so no engine runs it. `tls_upgrade` is the mechanism that ENG-20 built and the agent will serve (AGT-14).
- One port per scenario (8443 to 8446), so the realism labels tell the scenarios and their profiles apart by `server_port`.
- The client builds a fresh connector per session. A shared connector resumes TLS 1.3 sessions, and a resumed ClientHello carries `pre_shared_key`, which changes the JA4. A connector with resumption disabled leaves out `session_ticket` (0x0023), which also changes the JA4.

## Pitfalls

- The server profile names `server-tls13` and `server-tls12` exist only in the registry of `tests/common/tls.rs`. The agent must define the same names (AGT-14).

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-module-http1   # all pass
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-0c91ff | claim | none | Claimed #171 |
