# Handoff MOD-06 HTTP/2 module

| Field | Value |
|---|---|
| Issue | #173 |
| Branch | `wp/MOD-06` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (5 of 6 plan steps) |
| Current holder | claude-6d06a1, lease until 2026-11-10T06:15:03Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship the http2 module scenarios for prior knowledge cleartext HTTP/2 and for TLS with ALPN h2: concurrent streams with interleaved DATA, realistic SETTINGS, flow control with WINDOW_UPDATE, PING, a graceful GOAWAY, and server push disabled. The manifest carries the functions list of `print_catalog`, `mk/mod-http2.mk` and a workflow run the 60 second fuzz test, and a fixture capture with keys is scored: tshark parses every frame without errors, and nDPI classifies at least 99% of the sessions as HTTP/2 (cleartext directly, TLS through the decrypted view).

## Plan

- [x] 1. Manifest with the 20 functions and its parameters, four scenarios
- [x] 2. Tests: scenario frame checks, golden transcripts, https checks, `h2` crate replay in both directions
- [x] 3. Push the first checkpoint
- [x] 4. `fixture_peer`, `capture.sh`, `expected.yaml`, `decrypted_check.py`, record the capture
- [x] 5. `mk/mod-http2.mk`, `.github/workflows/mod-http2.yml`, README, `docs/wiki/Modules.md`
- [ ] 6. Log entry, checks, pull request, CI

## Done and verified

- `cargo test -p fps-module-http2 --test scenarios --test https --test transcript --test interop --test catalog` passes.

## Remaining

Step 6: pull request and CI. Open: nDPI 4.2.0 has no HTTP/2 protocol, the workflow builds nDPI tag 4.14 (unverified).

## Next action

Open the pull request, wait for CI, fix failures inside the package paths.

## Blockers and open questions

none

## Decisions taken in this package

- Four scenarios, one port each: `h2c_browse` 8080, `h2c_upload` 8081, `https_browse` 8443, `https_upload` 8444.
- No scenario variable can compute with other variables, and a flow cannot branch on what it reads, so stream ids are literals (odd streams 1, 3 to 13 for browsing, 1 and 3 for uploads) and every count of frames comes from a variable or from `http2.data_frames`.
- The TLS scenarios name the server profile `server-tls13-h2`, which the agent does not have yet.

## Pitfalls

- The TLS session of `fps-module-tls` fails with "received plaintext buffer full" when the receiver has more than 16 KiB of undelivered plaintext after a chunk of ciphertext (rustls limit). The https scenarios therefore run with small bodies in tests and in the fixture.

## Verify the current state

```bash
CARGO_BUILD_JOBS=1 cargo test -p fps-module-http2
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T04:45Z | claude-6d06a1 | claim | none | Claimed #173, pushed the handoff file |
| 2026-11-10T05:10Z | claude-6d06a1 | checkpoint | see git log | Manifest, scenarios, and tests |
