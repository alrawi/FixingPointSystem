# Handoff TRN-16 Per profile captures scored with the transport check

| Field | Value |
|---|---|
| Issue | #405 |
| Branch | `wp/TRN-16` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-80bdc1, lease until 2026-11-10T18:55:30Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`tests/netns/profiles.sh` records one client capture per OS profile and nothing scores it. Make it score each capture with `python3 -m realism score --os-profile NAME` (strict off for windows11 and macos14), so that a capture recorded with the wrong profile fails, and explain or fix why the harness captures show window scale 10 where linux6 expects 7.

## Plan

- [ ] 1. Reproduce the harness window scale locally in a namespace and explain it.
- [ ] 2. Add the scoring step to profiles.sh and a negative test (wrong profile fails).
- [ ] 3. Run the capture test locally if the sandbox allows, run the checks.
- [ ] 4. Log entry with the shared page notes, PR, CI.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Reproduce the window scale in a namespace.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
make lint-trn test-trn
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T17:26Z | claude-80bdc1 | claim | none | claimed #405 |
