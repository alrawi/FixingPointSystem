# Handoff TRN-05 TLS client fingerprint profiles

| Field | Value |
|---|---|
| Issue | #160 |
| Branch | `wp/TRN-05` |
| Pull request | #329 |
| Status | in progress |
| Progress | 90% (4 of 5 plan steps) |
| Current holder | claude-t05a1c, lease until 2026-11-09T21:23:49Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Define at least four browser like client profiles (Chrome, Firefox, Safari, Edge) in `modules/tls`, each with documented sources for its field orders and an expected JA4 in the profile definition. Where rustls cannot reproduce a field, document the gap and record both the JA4 that this implementation produces and the JA4 of the real browser. A CI test generates a ClientHello per profile through the real TLS module, computes JA4 with `realism/ja4.py`, and asserts it equals the expected value.

## Plan

- [x] 1. Claim commit and handoff
- [x] 2. Profile knob `sig_schemes` and the four profiles with documented sources (`native/src/browser.rs`)
- [x] 3. CI test `modules/tls/tests/ja4.rs` with the stdlib JA4 helper script and a tshark cross check
- [x] 4. Transport wiki page, module README, log entry
- [ ] 5. Checks, pull request, CI

## Done and verified

2 and 3: `cargo test -p fps-module-tls` passes (10 unit, 2 capture, 4 ja4, 14 sessions tests) with tshark 4.2.2 installed, so the tshark half ran. `cargo clippy -p fps-module-tls --all-targets -- -D warnings` and `cargo fmt --check` pass. 4: `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Open the pull request, wait for CI, then set the handoff to ready for review.

## Next action

Open the pull request and poll CI.

## Blockers and open questions

none. Owner decision for later, outside this package: a browser accurate JA4 needs another TLS stack or a patched rustls, see the log entry.

## Decisions taken in this package

- `expected_ja4` is what the implementation produces, and `browser_ja4` is the real browser value, so the profiles never claim a match that the wire does not show. A test asserts that they differ.
- The Python JA4 helper reads the ClientHello bytes with `realism/ja4.py`, so the test needs only `python3` and runs in the `rust` job of `ci.yml` without a workflow change. The tshark cross check needs the `mod-tls` workflow step named in the log entry.
- No dependency was added. The signature scheme order reorders the `mapping` of the ring provider and leaks one slice per distinct list, cached.

## Pitfalls

- Ring has no HPKE, so ECH GREASE (0xfe0d) is impossible, and certificate compression needs a decompressor, so no profile sends them.
- The Firefox and Safari suite and signature scheme orders are recalled from public captures and not re-verified, because fetches of capture pages failed in the sandbox. The wiki says so.

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=1 cargo test -p fps-module-tls
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T19:24Z | claude-t05a1c | claim | none | lease taken on #160 |
| 2026-11-09T20:08Z | claude-t05a1c | checkpoint | none | profiles, tests, wiki |
