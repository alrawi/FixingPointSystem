# Handoff MOD-08 IMAP module

| Field | Value |
|---|---|
| Issue | #175 |
| Branch | `wp/MOD-08` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-a11910, lease until 2026-11-09T19:43:23Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship the `imap` module under `modules/imap/`: client and server scenarios for CAPABILITY, LOGIN, SELECT and EXAMINE, FETCH of headers and full messages with literals from the content module, SEARCH, STORE, and LOGOUT, with correct tag sequencing and continuation responses. Plain IMAP on port 143. nDPI must classify at least 99% of the fixture sessions as IMAP.

## Plan

- [ ] 1. Native crate `fps-module-imap` with the template functions and unit tests
- [ ] 2. Manifest, six scenarios, catalog test
- [ ] 3. Scenario tests and golden transcripts
- [ ] 4. Fixture peer, capture.sh, expected.yaml, committed capture
- [ ] 5. mk file, workflow, README, Modules page, log entry
- [ ] 6. Checks, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write `modules/imap/native/`.

## Blockers and open questions

none

## Decisions taken in this package

Modelled on `modules/smtp` (MOD-07). Details are added as they are taken.

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-module-imap
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T18:13Z | claude-a11910 | claim | none | Claimed #175 and created the branch |
