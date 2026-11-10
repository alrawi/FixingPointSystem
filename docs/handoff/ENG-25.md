# Handoff ENG-25 int_bytes and now in the engine

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #375 |
| Branch | `wp/ENG-25` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-65883d, lease until 2026-11-10T14:16:38Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement the core template functions `int_bytes(value, width, endianness)` and `now(format)` of scenario contract v0.6 (CON-19) in `engine/fps-flow` and `engine/fps-interp`, with a clock seam that tests pin, and add parity fixtures to `verdicts.yaml`. Unit tests cover every width, both byte orders, the error cases, and the three `now` formats under a pinned clock.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Clock seam, `int_bytes`, `now` in the fps-flow template compiler and renderer, with unit tests
- [ ] 2. Parity fixtures in `verdicts.yaml` and the fixture scenarios
- [ ] 3. fps-interp: clock seam in `Session` and `ScenarioInstance`, variables across `after` test, interp tests
- [ ] 4. Checks, log entry, Engine page, PR

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Implement step 1.

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
| 2026-11-10T13:47Z | claude-65883d | claim | none | claimed #375 |
