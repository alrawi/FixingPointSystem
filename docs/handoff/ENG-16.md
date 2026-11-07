# Handoff ENG-16 Profiles and distributions in the template engine

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #230 |
| Branch | `wp/ENG-16` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-eng16a, lease until 2026-11-07T18:51:39Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

fps-flow's template parser and evaluator accept `profiles.<name>` and `dist.pareto|lognormal|exponential|uniform` with the argument rules of CON-10, so `web_profile.yaml` parses.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Parser, checker, evaluator for dist and profiles
- [ ] 2. Loader folds scenario profiles into templates
- [ ] 3. Unit and example tests
- [ ] 4. Engine wiki page, log entry, PR, CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Implement step 1 in engine/fps-flow/src/template/.

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
| 2026-11-07T18:41Z | claude-eng16a | claim | - | claimed #230, branch wp/ENG-16 from main |
