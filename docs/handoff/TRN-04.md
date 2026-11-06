# Handoff TRN-04 TLS transport module

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #159 |
| Branch | `wp/TRN-04` |
| Pull request | #227 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-t04b01, lease until 2026-11-06T23:01:05Z, renewed at each checkpoint |
| Last verified commit | 178a1d6, CI green on PR #227 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Wrap sessions in TLS 1.2 and 1.3 with rustls, with client profiles (SNI, ALPN, versions, groups, cipher order), per run server certificates from a generated authority, resumption, and NSS key log export. Acceptance: handshake rates within 10% of the S2 spike, and tshark decrypts a sample capture with the exported keys.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Crate skeleton, authority, client and server profiles, resumption
- [x] 2. Async session wrapper over SessionIo, handshake reports, key log
- [x] 3. Unit and loopback tests for versions, ALPN, SNI, groups, ciphers, resumption
- [x] 4. Handshake rate benchmark against the S2 results
- [x] 5. Capture and key log test with tshark check
- [x] 6. README, wiki page, log entry, checks, pull request

## Done and verified

Steps 4 to 6: `modules/tls/bench/run_s2.sh` style run, three alternating rounds with the S2 spike: lowest median ratio 0.922 over 30 figures, all within 5% of phase 0. Raw files in `modules/tls/results/phase2-sandbox/`. Wiki section, README, and log entry written; `plan.py check`, `render --check`, `lint`, `cargo fmt --check`, and clippy pass.

Steps 1 to 3. `cargo test -p fps-module-tls --offline` passes 7 unit tests, 14 session tests (versions, ClientHello order of ciphers, groups, ALPN, SNI, resumption on 1.2 and 1.3, alert names, key log lines, one byte reads, abort, kernel TCP), and 2 capture tests. The Python capture test decrypts a TLS 1.3 sample capture with the exported keys and fails with a corrupted key log. `cargo clippy -p fps-module-tls --all-targets -- -D warnings` is clean.

## Remaining

Coordinator review. The tshark test skips in the rust job because tshark is not installed there, see the shared page notes of the log entry for the workflow that completes it.

## Next action

Review and merge PR #227, then add the tshark workflow named in the log entry.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- The session wrapper is hand written over rustls::Connection and not tokio-rustls, to keep the dependency list to rustls, rcgen, and thiserror.
- The last handshake flight flushes fully, except the TLS 1.3 server tickets, which flush best effort. A full wait there deadlocks two peers over a transport with less buffer than the tickets.
- A transport that ends without close_notify reads as end of stream.

## Pitfalls

The test `large_transfers_survive_one_byte_reads_and_back_pressure` found the ticket deadlock above. Do not use pkill in this sandbox, the classifier denies it. `cargo build --example X -p other` builds examples only, so build the spike binary in its own command.

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
| 2026-11-06T22:30Z | claude-t04b01 | finish | 178a1d6 | PR #227 opened, all 19 check runs green, handed to review |
