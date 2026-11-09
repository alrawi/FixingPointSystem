# Handoff AGT-08 Mutual TLS and enrollment in the agent

| Field | Value |
|---|---|
| Issue | #252 |
| Branch | `wp/AGT-08` |
| Pull request | none yet |
| Status | in progress |
| Progress | 5% (0 of 5 plan steps) |
| Current holder | claude-a8c3f1, lease until 2026-11-09T05:22:20Z |
| Last verified commit | none |

## Goal

The agent dials the controller over mutual TLS (rustls), enrolls with a one time token over the HTTPS enrollment endpoint when it has no certificate, and renews its certificate at `renew_at`. `http://` keeps working against a controller in `insecure_plain_grpc` mode. The owner chose the HTTPS enrollment endpoint, so no contract changes.

## Plan

- [ ] 1. Identity module: key and CSR, enrollment and renewal over HTTPS, trust from a CA file or a pinned fingerprint, atomic files.
- [ ] 2. TLS channel in the session, permanent errors for `UNAUTHENTICATED` and handshake failures, renewal task, flags in `main.rs`.
- [ ] 3. Rust tests (fake enrollment server, permanent errors, exit codes).
- [ ] 4. `controller_e2e.sh` over mutual TLS with a token, plus a handshake failure case.
- [ ] 5. Wiki page `Agent.md`, log entry, checks, pull request.

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Write `engine/fps-agent/src/identity.rs`.

## Blockers and open questions

none

## Decisions taken in this package

(none yet)

## Pitfalls

(none yet)

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-AGT-08 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T03:52Z | claude-a8c3f1 | claim | none | Claimed #252 |
