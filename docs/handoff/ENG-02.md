# Handoff ENG-02 Scenario loader and semantic checks

| Field | Value |
|---|---|
| Issue | #22 |
| Branch | `wp/ENG-02` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-eng-02-ac55, lease until 2026-11-03T13:08:16Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Load scenario YAML into the program model with typed errors that name the file, flow, side, and action index, and apply the semantic rules of `tools/contracts/validate.py` for flows, `after`, `spawn`, `branch`, `last`, and module calls against the manifest `functions` catalog. One shared fixture directory feeds both validators, and every fixture gets the same verdict from Rust and Python.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Typed error location (`Location` with file, flow, side, action path) in place of the string `at`
- [ ] 2. Semantic rules for flows, after, spawn, branch, delay, expect datagram limits, and `last` placement
- [ ] 3. Module call check against the manifest `functions` catalog
- [ ] 4. Shared fixtures under `engine/fps-flow/tests/fixtures/` with a Rust test and a pytest that runs validate.py on them
- [ ] 5. Fix the bug #90 test helper, Engine page section, log entry

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
| 2026-11-03T11:39:02Z | claude-eng-02-ac55 | claim | none | Started from origin/main f4b9637 |
