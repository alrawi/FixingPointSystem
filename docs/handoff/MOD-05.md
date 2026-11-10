# Handoff MOD-05 HTTP/2 framing and HPACK functions

| Field | Value |
|---|---|
| Issue | #172 |
| Branch | `wp/MOD-05` |
| Pull request | #0 |
| Status | in progress |
| Progress | 50% (3 of 6 plan steps) |
| Current holder | claude-5d1c7a, lease until 2026-11-10T02:38:09Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement the HTTP/2 frame and HPACK functions that the http2 scenarios of MOD-06 call, as a native crate `fps-module-http2` under `modules/http2/native/`. Encoded frames must decode with the RFC 7541 Appendix C vectors and with the `h2` crate, and fuzz tests must run for 60 seconds without a panic.

## Plan

- [x] 1. Crate skeleton, frame encoder and parser (RFC 9113), catalog, argument binding
- [x] 2. HPACK tables, Huffman code, decoder, and encoder (RFC 7541)
- [x] 3. Template functions (`http2.*`) on top of the frame and HPACK code, with unit tests
- [ ] 4. RFC 7541 Appendix C vectors, frame round trips, `h2` crate interop, catalog test
- [ ] 5. Fuzz tests (short run in CI, 60 s run marked `#[ignore]`) and the lints
- [ ] 6. Modules page, log entry, pull request, CI

## Done and verified

- Steps 1 to 3. `cargo test -p fps-module-http2 --lib` passes 41 tests (frames, HPACK, Huffman including the Appendix C strings, and the template functions).

## Remaining

Steps 4 to 6.

## Next action

Write `modules/http2/native/tests/rfc7541.rs` with the Appendix C.2 to C.6 vectors.

## Blockers and open questions

none

## Decisions taken in this package

- The crate is `fps-module-http2`, module name `http2`, so templates call `http2.headers(...)`.
- `modules/http2/manifest.yaml` belongs to MOD-06. The catalog test compares the native catalog with the manifest when the manifest exists, and checks the catalog on its own until then.

## Pitfalls

- `Cargo.lock` gains the entry of the new crate and nothing else, because every dev dependency is already in the lock.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-MOD-05 CARGO_BUILD_JOBS=1 cargo test -p fps-module-http2
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T02:08Z | claude-5d1c7a | claim | none | Claimed #172 with 6 plan steps |
| 2026-11-10T02:40Z | claude-5d1c7a | checkpoint | ec8adfa | Steps 1 to 3 done |
