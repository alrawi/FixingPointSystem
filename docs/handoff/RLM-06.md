# Handoff RLM-06 Temporal distribution check

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #188 |
| Branch | `wp/RLM-06` |
| Pull request | #188 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-9c41e7, lease until 2026-11-10T02:39:45Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Compare session inter-arrival times and object sizes with target distributions given in the labels, through the KS test with alpha as a parameter. Samples from the target pass and samples from a shifted distribution fail. Opt-in check, report format 0.7.0.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Pure Python KS (one and two sample, exact and asymptotic p value) with tests against known values
- [ ] 2. Labels section `temporal` and its schema, target distributions
- [ ] 3. The check, report schema entry, CLI flags, metrics
- [ ] 4. Tests with direct samples and pcapgen captures
- [ ] 5. Docs: Realism.md, README, log entry, CI checks

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Step 1: write realism/ks.py.

## Blockers and open questions

none

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
| 2026-11-10T02:10Z | claude-9c41e7 | claim | | claimed #188 |
