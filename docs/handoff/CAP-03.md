# Handoff CAP-03 Scrubbing and provenance

| Field | Value |
|---|---|
| Issue | #195 |
| Branch | `wp/CAP-03` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-f353e8, lease until 2026-11-10T06:14:57Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Remove personal data and credentials from extracted sessions before scenario generation, and write the provenance record that module manifests require. `python3 -m capture scrub` reads a sessions document with payloads and writes a scrubbed one. `scrubbed: true` is set only from that scrub record. A seeded capture with known secrets yields output (sessions JSON and generated module) that contains none of them.

## Plan

- [ ] 1. Claim commit, handoff
- [ ] 2. Scrubber core (`capture/scrub.py`): addresses, protocol credentials, tokens, emails, names
- [ ] 3. CLI `scrub`, provenance in the manifest, scenario reads the scrub record
- [ ] 4. Seeded capture tests (raw, base64, percent decoded search), lint
- [ ] 5. Wiki page, README, log entry, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write `capture/scrub.py`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
make lint-cap test-cap PYTHON=python3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T04:45Z | claude-f353e8 | claim | see git log | claimed #195, branch wp/CAP-03 |
