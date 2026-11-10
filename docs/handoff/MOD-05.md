# Handoff MOD-05 HTTP/2 framing and HPACK functions

| Field | Value |
|---|---|
| Issue | #172 |
| Branch | `wp/MOD-05` |
| Pull request | #0 |
| Status | in progress |
| Progress | 83% (5 of 6 plan steps) |
| Current holder | claude-5d1c7a, lease until 2026-11-10T02:38:09Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement the HTTP/2 frame and HPACK functions that the http2 scenarios of MOD-06 call, as a native crate `fps-module-http2` under `modules/http2/native/`. Encoded frames must decode with the RFC 7541 Appendix C vectors and with the `h2` crate, and fuzz tests must run for 60 seconds without a panic.

## Plan

- [x] 1. Crate skeleton, frame encoder and parser (RFC 9113), catalog, argument binding
- [x] 2. HPACK tables, Huffman code, decoder, and encoder (RFC 7541)
- [x] 3. Template functions (`http2.*`) on top of the frame and HPACK code, with unit tests
- [x] 4. RFC 7541 Appendix C vectors, frame round trips, `h2` crate interop, catalog test, interpreter flow test
- [x] 5. Fuzz tests (short run with every `cargo test`, 60 s run marked `#[ignore]`) and the lints
- [ ] 6. Modules page, log entry, pull request, CI

## Done and verified

- Steps 1 to 5. `cargo test -p fps-module-http2` passes 44 unit tests, 9 RFC 7541 Appendix C tests, 2 `h2` interop tests, 5 catalog tests, 2 interpreter flow tests, and 2 short fuzz tests. `cargo clippy -p fps-module-http2 --all-targets -- -D warnings` and `cargo fmt --check` are clean.
- The 60 s fuzz run (`cargo test -p fps-module-http2 --release --test fuzz -- --ignored fuzz_60s`) ran 461350 rounds without a panic.

## Remaining

Step 6, and the plan, render, and lint checks of the repository.

## Next action

Update `docs/wiki/Modules.md`, write the log entry, run the repository checks, and open the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- The crate is `fps-module-http2`, module name `http2`, so templates call `http2.headers(...)`.
- `modules/http2/manifest.yaml` belongs to MOD-06. The catalog test compares the native catalog with the manifest when the manifest exists, and checks the catalog on its own until then. `cargo run -p fps-module-http2 --example print_catalog` prints the `functions:` list for the manifest.
- The module is stateful (one HPACK encoder and decoder per connection side), so `engine/fps-agent/src/modules.rs` needs a registration that is not `stateless!`. The agent clones the registry value for every session and flow side.
- A flow reads frames with two `expect` actions (9 octet header, then `bytes: "{{ http2.frame_length(last) }}"`), `set` of decimal values, and payload decoders that take `last` plus the flags. `tests/data/h2c_flow.yaml` shows the idiom.

## Pitfalls

- `Cargo.lock` gains the entry of the new crate, which lists `fps-interp` and `fps-io` as dev dependencies, and nothing else.
- `set` renders to lossy text, so frames and binary payloads must stay in `last` and never go into a variable.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-MOD-05 CARGO_BUILD_JOBS=1 cargo test -p fps-module-http2
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T01:08Z | claude-5d1c7a | claim | none | Claimed #172 with 6 plan steps |
| 2026-11-10T01:21Z | claude-5d1c7a | checkpoint | ec8adfa | Steps 1 to 3 done |
| 2026-11-10T01:38Z | claude-5d1c7a | checkpoint | pending | Steps 4 and 5 done |
