# Handoff MOD-12 SMB2 encoding functions

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #179 |
| Branch | `wp/MOD-12` |
| Pull request | #0 |
| Status | in progress |
| Progress | 83% (5 of 6 plan steps) |
| Current holder | claude-281119, lease until 2026-11-10T15:53:05Z |
| Last verified commit | see session log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement SMB2 message encoding and parsing for negotiate, session setup, tree connect, create, read, write, and close as a native Rust crate `fps-module-smb2` under `modules/smb2/native/`, with synthetic NTLM exchanges. Acceptance: tshark parses every encoded message without errors.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Crate skeleton, args, catalog
- [x] 2. SMB2 header, negotiate, close, and other encoders
- [x] 3. Synthetic NTLM (NEGOTIATE, CHALLENGE, AUTHENTICATE) and session setup
- [x] 4. Readers and golden bytes tests
- [x] 5. Fuzz, catalog test, tshark script
- [ ] 6. Log entry, Modules page, checks, PR, CI

## Done and verified

- Crate `fps-module-smb2` (`modules/smb2/native/`): `wire.rs` (header and bodies), `ntlm.rs` (NTLMSSP, NTLMv2, SPNEGO), `read.rs` (readers), `catalog.rs` (37 functions), `lib.rs` (dispatch).
- `cargo test -p fps-module-smb2`: tshark (4 sessions, SMB 2.0.2, 2.1, 3.0.2, 3.1.1, tshark 4.2.2), ntlm (MS-NLMP 4.2 vectors), messages (golden, round trips, every prefix), catalog (6), flow (2), fuzz_short all pass.
- `cargo test -p fps-module-smb2 --release --test fuzz -- --ignored fuzz_60s`: 280000 rounds in 60 s, no panic.
- `cargo clippy -p fps-module-smb2 --all-targets -- -D warnings` and `cargo fmt -p fps-module-smb2 -- --check` pass.
- `python3 tools/plan/plan.py check`, `render --check`, `lint` pass.

## Remaining

Open the pull request, wait for CI, and set the handoff to ready for review. `cargo deny check` could not run here (tool not installed); the `licenses` job of CI is the check. The workflow and mk file for the module are in the Shared page notes of the log entry, not in this package.

## Next action

Open the pull request against main and poll its check runs.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- The module has no state. Ids are arguments, and a session id travels as a two's complement integer because template integers are signed.
- Every writer emits the 4 octet direct TCP transport header, and every reader takes a message with or without it.
- A flow has no arithmetic and no loop counter, so the crate adds `add` and `chunks`. A `set` keeps text, so `hex` and `unhex` carry tokens in variables.
- The NTLM exchange sends no MIC and no key exchange, so the session key equals the session base key and `sign` can use it directly (HMAC-SHA256, SMB 2.x only).
- QUERY_DIRECTORY, TREE_DISCONNECT, and LOGOFF are included beyond the seven commands of the acceptance text, because MOD-13 needs them for browsing and teardown.

## Pitfalls

- The sandbox has tshark 4.2.2 at /usr/bin/tshark, although the brief says it has none.
- The worktree guard refuses a command that mixes git with other tools, or uses `..` paths, in one call.
- rustfmt reformats `function!(...)` macro calls oddly, so run `cargo fmt -p fps-module-smb2` before checking.

## Verify the current state

```bash
cargo test -p fps-module-smb2
cargo clippy -p fps-module-smb2 --all-targets -- -D warnings
```

Expected: every test passes, and the tshark test runs when tshark is installed.

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10 | claude-281119 | claim | | Claimed #179 |
| 2026-11-10 | claude-281119 | checkpoint | 77ca7f5 | Crate, NTLM, readers, tshark test |
| 2026-11-10 | claude-281119 | checkpoint | see PR | Golden, catalog, flow, fuzz tests, log entry, Modules page |
