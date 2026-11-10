# Handoff AGT-14 TLS profiles, TLS key export, and labeled criteria in the agent

| Field | Value |
|---|---|
| Issue | #324 |
| Branch | `wp/AGT-14` |
| Pull request | #357 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-a14c3e, lease until 2026-11-10T02:37:47Z |
| Last verified commit | 9727f6f |

## Goal

The agent builds the TLS profiles that a plan's `tls_upgrade` actions name, ships the TLS key log in the capture file when `export_tls_keys` is set, and applies `Criterion.labels` (at least `side`). HTTPS runs through the real agent in the controller e2e script, a capture of an https run decrypts with the exported keys, and a server side criterion counts the server series.

## Plan

- [x] 1. TLS profiles: scan the scenarios, refuse unknown names at prepare, build `TlsProfiles` with a seed derived authority (`src/tls.rs`, `RunAuthority::from_pkcs8_der`)
- [x] 2. Key tap and the secrets block in the joined capture (`src/control.rs`)
- [x] 3. Criterion labels (`src/criteria.rs`, `src/plan.rs`, `src/convert.rs`)
- [x] 4. Tests (`tests/tls.rs`), the https runs in `tests/controller_e2e.sh`, docs and log entry
- [x] 5. CI green, then review

## Done and verified

- `cargo test -p fps-agent --lib` and `--test tls` pass locally (8 tests in `tls.rs`), `cargo clippy -p fps-agent -p fps-module-tls --all-targets -- -D warnings` is clean.
- `bash -n tests/controller_e2e.sh`. The script runs only in CI (it needs the Go controller, which cannot build here).

## Remaining

none. CI is green on 9727f6f. `module (http1)` failed once with 4 tshark parser errors in a live recorded plain http capture and passed on rerun (flake, unrelated paths).

## Next action

none

## Blockers and open questions

none

## Decisions taken in this package

- Trust: both sides derive the CA key from the run seed (P-256, SHA-256 of a label and the seed, public point with `num-bigint`). `RunAuthority::from_pkcs8_der` is the one change outside the catalog paths (`modules/tls/native/src/authority.rs`), because `ServerIdentity` can only come from a `RunAuthority` and `generate` is random. Ed25519 would be simpler to derive but the Chrome and Safari profiles do not list `ed25519`, so they could not verify a leaf that an Ed25519 CA signed.
- One connector per session: `fps_runtime::TlsProfiles` shares a connector per profile, so the agent disables tickets and sessions on its server profiles. No client then holds anything to resume. A follow up in ENG could make `TlsProfiles` build a connector per session.
- Key log: embedded decryption secrets block in the joined pcapng, one per section after the interface description, only with `export_tls_keys`. The TLS layer cannot name the session of a handshake, so the lines are plan wide while a capture runs.
- Labels: a series counts when it carries every label; keys must be catalog labels of the metric; `side` picks the side.

## Pitfalls

- `fps-capture/tests/common::packets` asserts rising timestamps, which fail for the agent's whole session writes. `tests/tls.rs` parses frames itself.
- The first party `http1` replaces the example `http1` in the e2e script, because both have the name and the version 0.1.0 and the controller registry holds one.

## Verify the current state

```bash
cargo test -p fps-agent --test tls
cargo test -p fps-agent --lib
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T01:55:00Z | claude-a14c3e | claim | none | claimed #324 |
| 2026-11-10T02:25:00Z | claude-a14c3e | finish | 9727f6f | PR 357, CI green |
