# Handoff CTL-49 A spawned flow counts with the session that opened it

| Field | Value |
|---|---|
| Issue | #527 |
| Branch | `wp/CTL-49` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-c49e7a, lease until 2026-11-12T21:01:47Z |
| Last verified commit | none |

## Goal

The run counts the FTP data connection and the RTP media stream as sessions of their own, so one ftp or sip_rtp scenario session reads as two, the bytes per session halve, and the calibration doubles their shares. Find where the split happens, make sessions and bytes per scenario session include the spawned flows (run metrics, calibration, run report), and remove FPS_GATE2_BASIS from qa-gate2.yml once the bytes basis passes.

## Plan

- [ ] 1. Find where a spawned flow becomes a session of its own (engine/agent counters, runs metrics, calibration, report)
- [ ] 2. Fix it inside controller/internal/runs/ and controller/internal/report/, or stop and report if it is outside
- [ ] 3. Tests, wiki entry, shared page notes
- [ ] 4. Remove FPS_GATE2_BASIS from qa-gate2.yml, open PR
- [ ] 5. CI green, review

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Locate the split.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

Go modules cannot be downloaded here, so Go code cannot build locally; CI is the build loop.

## Verify the current state

```bash
gofmt -l controller/internal/runs controller/internal/report   # expect no output
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T19:31Z | claude-c49e7a | claim | | Claimed #527 |
