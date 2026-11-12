# Handoff RLM-20 Expected parser findings in the realism labels

| Field | Value |
|---|---|
| Issue | #524 |
| Branch | `wp/RLM-20` |
| Pull request | none yet |
| Status | in progress |
| Progress | 25% (1 of 4 plan steps) |
| Current holder | claude worker, lease until 2026-11-12T21:01:37Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Session rules of the realism labels declare the Zeek weirds and tshark expert items that a deliberately malformed scenario must produce (issue #491). A declared finding does not count, a missing one and an undeclared one fail, and the report lists both.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Labels, check, schema, report format 0.19.0, tests.
- [ ] 2. README, Realism page, schema copies.
- [ ] 3. Log entry with the Module Development text.
- [ ] 4. Pull request, CI, review.

## Done and verified

Steps 1 to 3 are in the branch; `python3 -m pytest realism/tests` passes, `ruff` and `mypy` on `realism` pass.

## Remaining

Pull request, CI, review.

## Next action

Open the pull request against main and poll its CI.

## Blockers and open questions

none

## Decisions taken in this package

Declarations per rule and per item, first matching rule applies, an item is met by one finding in any session of its rule, expected findings come before the port collision rule, an item of a parser that did not run is unchecked and does not fail. See the log entry.

## Pitfalls

The dataclass field `missing` of the result already names the missing parsers, so the missing findings are `missing_findings`.

## Verify the current state


```bash
python3 -m pytest realism/tests/test_expected_findings.py
```


## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T20:00Z | claude worker | claim and work | none | claimed #524 |
