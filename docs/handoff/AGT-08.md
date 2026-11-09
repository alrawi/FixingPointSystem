# Handoff AGT-08 Mutual TLS and enrollment in the agent

| Field | Value |
|---|---|
| Issue | #252 |
| Branch | `wp/AGT-08` |
| Pull request | #272 |
| Status | in progress |
| Progress | 90% (4 of 5 plan steps) |
| Current holder | claude-a8c3f1, lease until 2026-11-09T05:38Z |
| Last verified commit | 259c5b1 |

## Goal

The agent dials the controller over mutual TLS (rustls), enrolls with a one time token over the HTTPS enrollment endpoint when it has no certificate, and renews its certificate at `renew_at`. `http://` keeps working against a controller in `insecure_plain_grpc` mode. The owner chose the HTTPS enrollment endpoint, so no contract changes.

## Plan

- [x] 1. Identity module: key and CSR, enrollment and renewal over HTTPS, trust from a CA file or a pinned fingerprint, atomic files.
- [x] 2. TLS channel in the session, permanent errors for `UNAUTHENTICATED` and handshake failures, renewal task, flags in `main.rs`.
- [x] 3. Rust tests (fake enrollment server and fake controller over mutual TLS, permanent errors, exit codes).
- [x] 4. `controller_e2e.sh` over mutual TLS with a token, plus a handshake failure case.
- [ ] 5. Wiki page `Agent.md`, log entry, checks, pull request.

## Done and verified

- `src/identity.rs` and the session and main changes: `cargo clippy -p fps-agent --all-targets -- -D warnings` and `cargo fmt --check` are clean.
- `cargo test -p fps-agent --test mtls`: 14 tests pass (enroll, pin, renew, refused token, wrong CA, refused client certificate, UNAUTHENTICATED, plain peer, binary exit code 2).

- Whole crate `cargo test -p fps-agent` passes. A throwaway Go program around the real `controller/internal/pki` package (built with the standard library only, then deleted) accepted the agent CSR, issued the certificate, showed a mutual TLS handshake with cn=local, and refused the agent after `Revoke`, where the agent exited with code 2 and `received fatal alert: BadCertificate`.
- `controller_e2e.sh` passes `bash -n` and shellcheck. It cannot run here (no Go modules), so the `controller-e2e` CI job is its test.

## Remaining

Step 5: open the PR and wait for CI. The e2e script cannot run here, because Go modules cannot be downloaded, so CI is its test loop.

## Next action

Open the pull request and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- Renewal reuses the key and runs at `renew_at` in any state, because `POST /renew` uses its own HTTPS connection and never touches the open session. The agent never redials to renew, so D12 item 12 holds, and the next dial reads the new files.
- A handshake failure and `UNAUTHENTICATED` end the agent at once with exit code 2 (a configuration fault), not after the redial limit.
- A pinned fingerprint without a CA file: the controller sends only its leaf, so the first enrollment accepts the endpoint certificate, then checks that `ca_pem` has the pinned fingerprint and that the leaf chains to it. The token is exposed to a rogue endpoint in that one case. A CA file avoids it.

## Pitfalls

- h2 drops the type of io errors, so a TLS 1.3 client certificate alert on an open connection reaches the agent as text only. `tls_failure` therefore also looks for `received fatal alert`.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-AGT-08 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T03:52Z | claude-a8c3f1 | claim | none | Claimed #252 |
| 2026-11-09T04:10Z | claude-a8c3f1 | checkpoint | 259c5b1 | Identity module, session wiring, flags, 13 mtls tests |
| 2026-11-09T04:30Z | claude-a8c3f1 | checkpoint | see branch | e2e script over mutual TLS, docs, log entry |
