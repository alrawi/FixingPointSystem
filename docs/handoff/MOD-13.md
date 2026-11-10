# Handoff MOD-13 SMB2 module

| Field | Value |
|---|---|
| Issue | #180 |
| Branch | `wp/MOD-13` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-9c41d7, lease until 2026-11-10T18:55:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship smb2 scenarios for file browsing, download, and upload, with client and server sides and configurable file sizes. nDPI must classify at least 99% of the fixture sessions as SMB. MOD-12 built the native functions crate `fps-module-smb2`; this package adds the manifest, the scenarios, the fixture, and `capture.sh`.

## Plan

- [ ] 1. `manifest.yaml` with the catalog of the crate and the parameter schema, and the three scenarios `browse`, `download`, `upload`.
- [ ] 2. Tests that run both sides of each scenario through the interpreter, and the golden transcripts.
- [ ] 3. The `fixture_peer` example, `capture.sh`, and `expected.yaml`.
- [ ] 4. Record the fixture in a network namespace, score it with nDPI, check how nDPI treats the 4 octet direct TCP header.
- [ ] 5. README, log entry, wiki page notes, checks, pull request, CI.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write `modules/smb2/manifest.yaml`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-module-smb2
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T18:25Z | claude-9c41d7 | claim | none | Claimed #180, branch wp/MOD-13 from origin/main |
