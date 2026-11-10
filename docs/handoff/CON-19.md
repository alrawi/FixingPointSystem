# Handoff CON-19 Contract gaps found by CAP-02 and RLM-06

| Field | Value |
|---|---|
| Issue | #362 |
| Branch | `wp/CON-19` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-51af6f, lease until 2026-11-10T06:15:04Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Close four contract gaps, all additive. CAP-02 found no integer to bytes function, no clock function, and no rule for variables across `after`. RLM-06 found that the test configuration has no arrival or size distributions, and that the metric catalog has no `all` value for `application` and no `both` value for `property`. The owner approved the contract change.

## Plan

- [ ] 1. Template functions `int_bytes` and `now` and the `after` variable rule: README, validator, tests, scenario schema description (scenario contract v0.6)
- [ ] 2. Arrival and size distributions in the test configuration schema (v0.3), validator rules, tests
- [ ] 3. Metric catalog 0.3.0: `all` for `application`, `both` for `property`
- [ ] 4. `contracts/changes/CON-19.md`, CHANGELOG, README, embedded copies, generated code, wiki Contracts page
- [ ] 5. Run the checks, open the PR, wait for CI

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Start step 1.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 -m pytest tools/contracts -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T04:48Z | claude-51af6f | claim | none | Claimed #362, branch from origin/main 7975e20 |
