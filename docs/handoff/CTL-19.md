# Handoff CTL-19 Optional token for the metrics endpoint

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #136 |
| Branch | `wp/CTL-19` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-d5763a, lease until 2026-11-05T20:20:40Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 item 2. `GET /metrics` stays open by default, and a configuration setting can require a bearer token for it.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Setting `metrics_token_file` and flag `-metrics-token-file`, loader, and constant time bearer middleware in `internal/app`
- [ ] 2. Wire it in `cmd/fps-controller` and add tests (open, required, wrong token)
- [ ] 3. Document in the Controller wiki page, log entry, and run the checks

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Step 1.

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
| 2026-11-05T19:50Z | claude-d5763a | claim | | Claimed #136 |
