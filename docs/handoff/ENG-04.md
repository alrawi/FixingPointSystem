# Handoff ENG-04 Flow program interpreter

| Field | Value |
|---|---|
| Issue | #24 |
| Branch | `wp/ENG-04` |
| Pull request | #98 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-eng-04-6d65, lease until 2026-11-03T13:08:31Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Execute compiled flow programs of `fps-flow` over any `fps_io::SessionIo` for TCP sessions, with timeouts, the three expect modes, and seeded delay distributions. Per session generators derive from the run seed and the session id, and every failure maps to a `reason` value of `contracts/metrics/catalog.yaml`.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Crate skeleton with dependencies, `session_rng`, and the `Reason` and `Failure` types
- [x] 2. Session runner with send, the three expect modes, timeouts, and `last`
- [x] 3. Delays of the five distributions, set, loop, branch, and the three close modes
- [x] 4. Client and server tests over the duplex, one test per reachable failure reason
- [x] 5. Engine page section, log entry, handoff, and pull request

## Done and verified

- Steps 1 to 4. `cargo test -p fps-interp` passes 34 tests (9 unit, 2 catalog, 13 failure, 10 session). `cargo clippy -p fps-interp --all-targets -- -D warnings` and `cargo fmt --check` are clean, and `cargo deny check` passes.
- Step 5. The Engine page has the section "The flow program interpreter", and `docs/wiki/log/2026-11-03T1152-ingest-eng-04-flow-program-interpreter.md` carries the shared page notes. `plan.py check`, `render --check`, and `lint` pass.

Acceptance. Client and server programs run against each other over `fps_io::memory::duplex` in `tests/sessions.rs` and `tests/failures.rs`. Each of the nine catalog reasons has a test in `tests/failures.rs` that triggers it.

## Remaining

- `tls_failure` is triggered through a test transport that reports `InvalidData`, because no TLS transport exists yet.
- `send` has no timeout, and the interpreter records no `transactions.*` or response time, see the log entry follow ups.

## Next action

Review the pull request #98.

## Blockers and open questions

none

## Decisions taken in this package

- `connect` and `accept` go through an `Opener` trait, so the interpreter applies the open timeout and maps `ConnectionRefused` and `TimedOut` to the catalog reasons.
- `session_rng` takes the side as a third input, so the client and server of one session draw from different streams.
- `accept` that does not complete within the open timeout also reports `connect_timeout`, since the session never opened.
- The drain timeout of `close: fin` reports `expect_timeout`, because it is a read that waits too long.
- `InvalidData` from the transport reports `tls_failure`, as `tokio-rustls` reports TLS errors that way.
- A `stop` future aborts the transport with a reset and reports `aborted`.
- Negative counts, negative fixed delays, and rejected distribution parameters report `internal`.
- `Failure` boxes its kind, so `Result<(), Failure>` stays below the 128 byte limit of `clippy::result_large_err` that CI clippy enforces.
- Failures report the catalog action index in document order with the opener at 0, not the compiled op index.

## Pitfalls

- `rand_distr` 0.4 accepts a negative lognormal sigma, so we check it ourselves.
- A peer that failed drops its transport, so the other side often fails a later send with `BrokenPipe`, which is `peer_closed`. Tests assert both sides.
- One byte duplex reads of 70000 byte bodies are slow in debug builds, so that test uses 700 bytes.

## Verify the current state

```bash
cargo test -p fps-interp
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T11:40:17Z | claude-eng-04-6d65 | claim | none | Claimed with card 6602808216 |
| 2026-11-03T11:46:00Z | claude-eng-04-6d65 | checkpoint | 3 of 5 | Session runner, delays, failure reasons |
| 2026-11-03T11:50:00Z | claude-eng-04-6d65 | checkpoint | 4 of 5 | Tests over the duplex |
| 2026-11-03T11:55:00Z | claude-eng-04-6d65 | finish | see PR | Ready for review |
