# Handoff MOD-11 SSH module

| Field | Value |
|---|---|
| Issue | #178 |
| Branch | `wp/MOD-11` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-3cddb6, lease until 2026-11-10T14:16:45Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship ssh scenarios with real key exchange through a native extension backed by a GPL-3.0 compatible SSH library. Deliverable: interactive shell and file copy shaped scenarios with configurable session length. Acceptance: the server and client complete key exchange in every fixture session, and nDPI classifies them as SSH.

## Plan

- [ ] 1. Choose the SSH library, check its license, and write the crate skeleton, manifest, and first scenario
- [ ] 2. Implement the native functions and the fixture peer with real key exchange
- [ ] 3. Write the scenarios (shell, file copy) and the interpreter, catalog, and transcript tests
- [ ] 4. Write capture.sh, expected.yaml, and the Zeek check
- [ ] 5. Write the README and the log entry with the shared page notes
- [ ] 6. Run the checks, open the PR, and wait for CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read modules/ftp and modules/pop3, then choose the SSH library.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
git log --oneline -3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T12:46Z | claude-3cddb6 | claim | none | Claimed issue 178, card 6767703439 |
