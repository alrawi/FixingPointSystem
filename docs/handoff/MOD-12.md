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
| Progress | 50% (3 of 6 plan steps) |
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
- [ ] 4. Readers and golden bytes tests
- [ ] 5. Fuzz, catalog test, tshark script
- [ ] 6. Log entry, Modules page, checks, PR, CI

## Done and verified

- Steps 1 to 3 (and the tshark test of step 4): crate `fps-module-smb2` (`modules/smb2/native/`) with `wire.rs` (header and bodies), `ntlm.rs` (NTLMSSP, NTLMv2, SPNEGO), `read.rs` (readers), `catalog.rs` (36 functions). `cargo test -p fps-module-smb2 --test tshark` passes with tshark 4.2.2 on four sessions (2.0.2, 2.1, 3.0.2, 3.1.1) and `--test ntlm` passes with the MS-NLMP 4.2.4 vectors.

## Remaining

Tests: messages (golden), catalog, fuzz, flow. Docs: log entry with Shared page notes, Modules page section, README of the crate if needed.

## Next action

Write tests/messages.rs (round trips, golden bytes), tests/catalog.rs, tests/fuzz.rs, tests/flow.rs, then the log entry and the Modules page row.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10 | claude-281119 | claim | | Claimed #179 |
