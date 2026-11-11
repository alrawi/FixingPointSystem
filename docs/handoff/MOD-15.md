# Handoff MOD-15 HLS video module

| Field | Value |
|---|---|
| Issue | #182 |
| Branch | `wp/MOD-15` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-fc5237, lease until 2026-11-11T05:13:47Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship `hls` scenarios that fetch a master playlist, a media playlist, and then segments at the segment cadence over a bitrate ladder. Deliverable: master and media playlists and synthetic segment bodies of the configured size. Acceptance: the temporal check accepts the segment fetch cadence, and the byte share matches the configured bitrate within 5%.

## Plan

- [ ] 1. Write the native crate (playlists, MPEG-TS segments, sizes, pacing) and the manifest with unit and catalog tests
- [ ] 2. Write the scenarios and the interpreter and golden transcript tests over an in memory network
- [ ] 3. Write `fixture_peer`, `capture.sh`, and record and score the fixture in the QA-01 harness
- [ ] 4. Write the cadence and byte share check with its tests
- [ ] 5. Write the README, the log entry with the shared page notes, and the Modules page section
- [ ] 6. Run the checks, open the PR, and wait for CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Step 1.

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
| 2026-11-10T17:13Z | claude-fc5237 | claim | none | Claimed #182 |
