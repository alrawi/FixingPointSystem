# Handoff ENG-01 Engine crate skeleton

| Field | Value |
|---|---|
| Issue | #21 |
| Branch | `wp/ENG-01` |
| Pull request | #71 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-eng01-1906, lease until 2026-11-02T21:46:57Z |
| Last verified commit | the head of `wp/ENG-01` at review |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Create the engine crates `fps-io`, `fps-flow`, `fps-interp`, and `fps-stats` under `engine/`. Move the session I/O seam and the program model and template parser of the S1 spike into `fps-io` and `fps-flow` with their tests. The crates `fps-interp` and `fps-stats` start empty with the lint header of the coding standards.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim, branch, handoff, and pull request
- [x] 2. `fps-io` with the `SessionIo` trait, abort, and the in memory duplex, with tests
- [x] 3. `fps-flow` with the program model and template parser ported from `spikes/s1-engine`, with tests
- [x] 4. Empty documented crates `fps-interp` and `fps-stats`
- [x] 5. `test-eng` Makefile block, workspace check, clippy, fmt, and cargo deny
- [x] 6. Wiki ingest, Log entry, final checks, and review

## Done and verified

Every step passed the commands below on Rust 1.97.0 and cargo-deny 0.20.2.

| Step | Command | Result |
|---|---|---|
| 2, 3 | `cargo test -p fps-io -p fps-flow` | 14 unit and 6 component, contract, and property tests in `fps-flow`, 6 unit and 3 component and property tests in `fps-io`, all pass |
| 4 | `cargo test -p fps-interp -p fps-stats` | builds, no tests |
| 5 | `cargo clippy --workspace --all-targets -- -D warnings` | pass, spike crates included |
| 5 | `cargo fmt --all -- --check` | pass |
| 5 | `cargo deny check advisories bans licenses sources` | advisories, bans, licenses, and sources ok; the `windows-sys` duplicate warning exists on `main` already |
| 5 | `make test-eng`, `make test-spk` | pass, the spike still builds and its tests pass |
| 6 | `make lint test` | pass |

## Remaining

Nothing inside the package. Gaps that later packages own are listed below.

- `DatagramIo` is absent, and ENG-06 adds it.
- The kernel TCP implementation of `SessionIo` is absent, because it belongs to the TRN backend crate.
- `LoadError` carries its location as a string, and ENG-02 replaces it with the file, flow, and action index.
- No ENG workflow file exists, because the `rust` job of `ci.yml` already runs fmt, clippy, and `cargo test --workspace` over the engine crates.

## Next action

Review pull request #71 against the Definition of Done.

## Blockers and open questions

None.

## Decisions taken in this package

- `SessionIo::abort` has no default body, so every transport states its reset behavior. The trait doc defines the semantics, and `MemoryIo` follows them, including `ConnectionReset` at the peer and `NotConnected` at the aborting side.
- `fps-io` holds no kernel socket code and depends only on Tokio with `io-util`.
- `fps-flow` records `l4` and accepts UDP flows, because the scenario schema allows them and the DNS example uses one. The spike rejected them.
- Errors use `thiserror` enums in place of `anyhow`, as Coding Standards require for libraries.
- Rendering is generic over `rand::Rng`, and `Filler` uses `rand_pcg::Pcg64Mcg`, which is portable. The spike used `SmallRng`, whose algorithm differs across platforms.
- A `clippy.toml` in `fps-io` and `fps-flow` allows `unwrap`, `expect`, and `panic` in unit tests only.
- The `test-eng` target derives the package list from `engine/*/Cargo.toml`, so later ENG crates need no Makefile edit.
- Spike S1 keeps its own `program.rs` and `template.rs`, so its benchmark harness and Gate 0 numbers stay reproducible. The duplication is recorded in the Engine wiki page.

## Pitfalls

- Building a `Filler` costs one random draw per byte of its 1 MiB buffer. A property test that builds one per case ran for 40 seconds, so the tests share one filler through `OnceLock`.
- CI installs the newest stable toolchain, whose clippy may carry lints that a local toolchain lacks. The first CI run failed on `clippy::chunks_exact_to_as_chunks`, and `decode_hex` now uses `as_chunks`.
- `cargo fmt --all` formats the spike crates too. They were already formatted, so it changed nothing there.

## Verify the current state

```bash
export CARGO_TARGET_DIR=$PWD/target
cargo test -p fps-io -p fps-flow        # every test passes
cargo clippy --workspace --all-targets -- -D warnings
make test-eng test-spk
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:04:31Z | claude-eng01-1906 | claim | | Card 6587196943 |
| 2026-11-02T20:16:57Z | claude-eng01-1906 | checkpoint | b734346 | Engine crates and Makefile target |
| 2026-11-02T20:19:00Z | claude-eng01-1906 | finish | | Wiki ingest, ready for review |
| 2026-11-02T20:24:00Z | claude-eng01-1906 | checkpoint | | Fix the CI clippy lint chunks_exact_to_as_chunks |
