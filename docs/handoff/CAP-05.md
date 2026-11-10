# Handoff CAP-05 Provenance fields from the scrub record

| Field | Value |
|---|---|
| Issue | #393 |
| Branch | `wp/CAP-05` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-5c05a1, lease until 2026-11-10T15:53:13Z |
| Last verified commit | none |

## Goal

CON-20 added the optional manifest provenance fields `capture_sha256`, `scrub_rules` and `scrubbed_at` (module manifest schema v0.4). The capture tools write the three fields from an intact scrub record, and refuse a record that does not match the capture. A test scrubs a committed fixture and validates the written provenance against schema v0.4 with literal values.

## Plan

- [ ] 1. Claim, handoff, branch.
- [ ] 2. `capture/scrub.py` and `capture/scenario.py`: write the three fields, refuse a record whose capture digest does not match the document.
- [ ] 3. Test on a committed fixture, `Capture.md`, log entry, checks, PR, CI.

## Done and verified

Nothing yet.

## Remaining

Everything after step 1.

## Next action

Implement step 2.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 -m pytest capture/tests -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T14:23Z | claude-5c05a1 | claim | none | Claimed #393 with card 6769813726 |
