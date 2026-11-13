# Handoff MOD-24 Packet gap labels in place of the module timing scripts

| Field | Value |
|---|---|
| Issue | #493 |
| Branch | `wp/MOD-24` |
| Pull request | #530 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-6afc7f, lease until 2026-11-12T21:01:58Z |
| Last verified commit | 85a9728 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Replace `rtp_check.py` (sip_rtp) and `hls_check.py` (hls) with `temporal.packets` labels (RLM-15), and remove the http2 TLS port wrapper that RLM-18 made redundant. Update the workflows. Acceptance: the sip_rtp, hls and http2 module realism jobs pass, and a jittered sip_rtp capture fails through the labels.

## Plan

- [x] 1. Add the labels, remove the sip_rtp script and the http2 wrapper, update capture scripts, mk files, workflows
- [x] 2. Hls decision (see findings) and the README and test updates of the three modules
- [x] 3. Module-Development.md, log entry, checks
- [x] 4. Open the PR, wait for CI, review

## Done and verified

Steps 1 to 3: sip_rtp labels (`ms: [20, 30]`, `tolerance_ms: 8`), `rtp_check.py` and its tests removed, `test_packet_labels.py` (4 tests, committed capture passes, jittered capture fails), http2 wrapper, its Rust test and `split_client_hello.py` removed, `capture.sh` of both modules, `mk/mod-sip.mk`, `mod-sip.yml`, READMEs, Modules.md, Module-Development.md, log entry. Verified locally: unittest of sip_rtp (11) and hls (7), `realism score` of sip_rtp with tshark and nDPI 4.2.0 passes, `realism decrypted` on http2 lists 40 of 40 TLS sessions without the wrapper (the nDPI HTTP2 step needs nDPI 4.14, CI only), plan check, render --check, lint, `make lint-mod-sip lint-mod-hls lint-mod-http2`.

## Remaining

Nothing. CI of PR #530 on 85a9728: every job green (module sip_rtp, hls, http2, harness, fuzz, contracts) except `rust`, which fails at `cargo fmt --check` on `modules/ldap/native/src/catalog.rs` and fails the same way on main (43c0b7b). The hls decision is open for the coordinator.

## Next action

Review PR #530.

## Blockers and open questions

none

## Decisions taken in this package

- hls keeps `hls_check.py` and gets no packet gap labels (see the log entry), because the labels cannot express its keepalive cadence or its byte checks. The coordinator decides whether to remove it anyway.
- `mk/mod-sip.mk` is edited although it is outside the package paths, because it called the removed script.

## Pitfalls

- `hls_check.py` also checks bytes (byte share, bit rate, segment body size), which no label covers, and the client stream of a keepalive connection mixes playlist and segment requests, so a constant 400 ms target fails.

## Verify the current state

```bash
git log --oneline origin/wp/MOD-24
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T19:36 | claude-6afc7f | claim | none | claimed issue #493 |
| 2026-11-12T20:00 | claude-6afc7f | finish | 85a9728 | PR 530 open, CI green except the rust fmt failure that main has too |
