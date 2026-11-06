# Handoff TRN-04 TLS transport module

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #159 |
| Branch | `wp/TRN-04` |
| Pull request | none yet |
| Status | in progress |
| Progress | 50% (3 of 6 plan steps) |
| Current holder | claude-t04b01, lease until 2026-11-06T23:01:05Z, renewed at each checkpoint |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Wrap sessions in TLS 1.2 and 1.3 with rustls, with client profiles (SNI, ALPN, versions, groups, cipher order), per run server certificates from a generated authority, resumption, and NSS key log export. Acceptance: handshake rates within 10% of the S2 spike, and tshark decrypts a sample capture with the exported keys.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Crate skeleton, authority, client and server profiles, resumption
- [x] 2. Async session wrapper over SessionIo, handshake reports, key log
- [x] 3. Unit and loopback tests for versions, ALPN, SNI, groups, ciphers, resumption
- [ ] 4. Handshake rate benchmark against the S2 results
- [ ] 5. Capture and key log test with tshark check
- [ ] 6. README, wiki page, log entry, checks, pull request

## Done and verified

Steps 1 to 3. `cargo test -p fps-module-tls --offline` passes 7 unit tests, 14 session tests (versions, ClientHello order of ciphers, groups, ALPN, SNI, resumption on 1.2 and 1.3, alert names, key log lines, one byte reads, abort, kernel TCP), and 2 capture tests. The Python capture test decrypts a TLS 1.3 sample capture with the exported keys and fails with a corrupted key log. `cargo clippy -p fps-module-tls --all-targets -- -D warnings` is clean.

## Remaining

Steps 4 to 6: run the handshake benchmark against the S2 spike, document the numbers, README, wiki page, log entry, checks, pull request. The tshark test skips here because tshark is missing.

## Next action

Build `cargo build --release -p fps-spike-tls -p fps-module-tls --example handshake_bench` with CARGO_BUILD_JOBS=1 and run both binaries alternately with `--duration 3`.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- The session wrapper is hand written over rustls::Connection and not tokio-rustls, to keep the dependency list to rustls, rcgen, and thiserror.
- The last handshake flight flushes fully, except the TLS 1.3 server tickets, which flush best effort. A full wait there deadlocks two peers over a transport with less buffer than the tickets.
- A transport that ends without close_notify reads as end of stream.

## Pitfalls

The test `large_transfers_survive_one_byte_reads_and_back_pressure` found the ticket deadlock above. Do not use pkill in this sandbox, the classifier denies it.

## Verify the current state

```bash
cargo test -p fps-module-tls --offline   # 7 + 14 + 2 tests pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T22:25Z | claude-t04a1f | claim | | Claimed #159 with 6 steps |
| 2026-11-06T23:00Z | claude-t04b01 | takeover | | Claimed #159 again after the stopped worker, branch had only the claim commit |
