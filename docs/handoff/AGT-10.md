# Handoff AGT-10 Shared capture cell and pinned enrollment check in the agent

| Field | Value |
|---|---|
| Issue | #276 |
| Branch | `wp/AGT-10` |
| Pull request | #307 |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-a10c3e, lease until 2026-11-09T17:41:43Z |
| Last verified commit | 45ef53e |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent uses `fps_capture::CaptureCell` for TCP and UDP, so `StartCapture` records TCP sessions. With `--tls-ca-sha256` and no CA file it checks the pinned CA in the presented chain during the enrollment handshake, before it sends the token. A refused client certificate under TLS 1.3 that shows only as a closed connection is a permanent TLS failure again, and `tests/mtls.rs` requires `Permanent`.

## Plan

- [x] 1. Shared capture cell (`control.rs` re-export, `run.rs` passes the cell to `RuntimeConfig`), test in `tests/run_control.rs`
- [x] 2. `PinnedVerifier` in `identity.rs`, fake endpoint tests in `tests/mtls.rs`
- [x] 3. Close after the handshake: `closed_before_response`, `Lost::ClosedAfterHandshake`, confirm by one immediate redial, tighten the refused certificate test
- [ ] 4. Docs (`Agent.md`), log entry, checks, PR, CI

## Done and verified

- Steps 1 to 3: `cargo test -p fps-agent` passes, `cargo clippy -p fps-agent --all-targets -- -D warnings` is clean. The refused certificate test ran 50 times alone and 100 times under CPU load with no failure.

## Remaining

PR and CI.

## Next action

Open the PR, then poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- A close after the handshake is permanent only on the second consecutive occurrence, with the second dial immediate, so one restart of the controller stays redialable. The refused certificate case still ends within two dials.
- The server runtime of a process that also runs clients gets an empty cell, so a connection is not recorded twice (ENG-19 finding).

## Pitfalls

- The disk of the sandbox was full, so builds used `CARGO_PROFILE_DEV_DEBUG=0` and the target directory of a finished package.

## Verify the current state

```bash
cargo test -p fps-agent --test mtls --test run_control
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T17:15Z | claude-a10c3e | claim | none | claimed #276 |
