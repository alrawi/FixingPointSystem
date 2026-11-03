# Handoff RLM-02 Parser error checks with Zeek and tshark

| Field | Value |
|---|---|
| Issue | #52 |
| Branch | `wp/RLM-02` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-rlm-02-f66d, lease until 2026-11-03T04:13:46Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Count parser errors and malformed packets per application with tshark and Zeek, each run as a separate process, and add the counts to the realism report as a new check. A capture with one malformed DNS message reports exactly one parser error.

## Plan

- [ ] 1. Parser runner interface with tshark and Zeek runners and stub runners
- [ ] 2. Parser error check that attributes errors to sessions and applications, with its report schema entry
- [ ] 3. Command integration and unit tests, including the malformed DNS acceptance test
- [ ] 4. Optional real tool tests, make targets, and the rlm workflow
- [ ] 5. README, component page, log entry, and pull request

## Done and verified

None yet.

## Remaining

Every plan step.

## Next action

Write the parser runner module in `realism/parsers.py`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
make lint-rlm test-rlm
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-rlm-02-f66d | claim | none | Session start, branch created from origin/main |
