# Handoff MOD-16 NTP module

| Field | Value |
|---|---|
| Issue | #183 |
| Branch | `wp/MOD-16` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-b74ad8, lease until 2026-11-11T05:13:57Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship ntp client and server scenarios in client and server modes, with NTPv4 packet encoding with realistic poll intervals. Acceptance: tshark parses every fixture packet without errors.

## Plan

- [ ] 1. Read the dns module, contracts, and the `now` builtin; design the manifest and scenarios
- [ ] 2. Native crate with the NTPv4 codec and template functions, with unit tests
- [ ] 3. Scenarios, interpreter tests, and golden transcripts
- [ ] 4. Fixture peer, capture.sh, fixtures, expected.yaml, and the realism score
- [ ] 5. README, wiki page and log entry with shared page notes, checks, PR, and CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read modules/dns and contracts/ for the `now` builtin.

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
| 2026-11-10T23:45Z | claude-b74ad8 | claim | none | Claimed #183 and created the handoff file |
