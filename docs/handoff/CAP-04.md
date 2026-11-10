# Handoff CAP-04 Profile extractor

| Field | Value |
|---|---|
| Issue | #196 |
| Branch | `wp/CAP-04` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-211a73, lease until 2026-11-10T14:16:51Z |
| Last verified commit | none |

## Goal

Derive an application mix and temporal models from a capture and write a test profile. Deliverable: mix shares, session arrival models, and object size models fitted per application. Acceptance: a profile extracted from a generated run reproduces its configured mix within 2 points.

## Plan

- [ ] 1. Read the sessions document, scrub record and fixtures; design the profile format
- [ ] 2. Implement classification, mix shares, arrival and size models
- [ ] 3. Add the `profile` command, tests with literal values from committed fixtures
- [ ] 4. Docs, log entry, checks, PR

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read `capture/sessions.py`, `capture/scrub.py`, and the committed fixtures.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
python3 -m pytest capture/tests -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T12:47Z | claude-211a73 | claim | none | Claimed #196 with card 6767705449 |
