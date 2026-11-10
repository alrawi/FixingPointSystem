# Handoff CAP-05 Provenance fields from the scrub record

| Field | Value |
|---|---|
| Issue | #393 |
| Branch | `wp/CAP-05` |
| Pull request | #395 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-5c05a1, lease until 2026-11-10T15:53:13Z |
| Last verified commit | 1178160 |

## Goal

CON-20 added the optional manifest provenance fields `capture_sha256`, `scrub_rules` and `scrubbed_at` (module manifest schema v0.4). The capture tools write the three fields from an intact scrub record, and refuse a record that does not match the capture. A test scrubs a committed fixture and validates the written provenance against schema v0.4 with literal values.

## Plan

- [x] 1. Claim, handoff, branch.
- [x] 2. `capture/scrub.py` and `capture/scenario.py`: write the three fields, refuse a record whose capture digest does not match the document.
- [x] 3. Test on a committed fixture, `Capture.md`, log entry, checks, PR, CI.

## Done and verified

Step 2 and the tests: `make lint-cap test-cap` passes (175 passed, 1 skipped). `plan.py check`, `render --check` and `lint` pass.

## Remaining

none

## Next action

none; the coordinator reviews PR #395.

## Blockers and open questions

none

## Decisions taken in this package

- `check_record` (shared with `capture profile`) carries the new refusal, so both commands treat a mismatched record the same way.
- A null `capture_sha256` in the record (document without `source.sha256`) is intact; the field is left out of the manifest.
- `notes` keeps the scrubber version, the sessions digest and the byte change; the fields replace the rest.

## Pitfalls

An unscrubbed capture manifest (`scrubbed: false`) is invalid under the schema on purpose, so tests validate the synthetic and scrubbed cases only.

## Verify the current state

```bash
python3 -m pytest capture/tests -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T14:23Z | claude-5c05a1 | claim | none | Claimed #393 with card 6769813726 |
| 2026-11-10T14:45Z | claude-5c05a1 | checkpoint | see git log | Code, tests, docs, log entry done; PR next |
| 2026-11-10T15:05Z | claude-5c05a1 | finish | 1178160 | CI green on PR #395 (10 checks); ready for review |
