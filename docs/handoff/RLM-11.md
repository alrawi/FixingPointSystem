# Handoff RLM-11 Pinned Zeek and visible parser skips

| Field | Value |
|---|---|
| Issue | #140 |
| Branch | `wp/RLM-11` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-7e41b9, lease until 2026-11-05T20:20:37Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 5 and 23. CI installs one pinned Zeek version from the Zeek repository on the openSUSE Build Service, set in one place for both workflows, and `--parsers auto` stays but the realism report and its text summary list every parser that it skipped.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. One pinned Zeek version file, read by `realism/ci/install-tools.sh`, which both workflows call
- [ ] 2. `skipped_parsers` in the report, the schema, the CLI message, and the CI summary rows, with tests
- [ ] 3. Realism wiki page and log entry
- [ ] 4. Pull request and green CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Write exactly one concrete step that the next agent can start immediately.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-05T19:50Z | claude-7e41b9 | claim | | Claimed #140 with 4 plan steps |
