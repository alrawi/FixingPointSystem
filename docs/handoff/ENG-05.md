# Handoff ENG-05 Vectored writes for fill bodies

| Field | Value |
|---|---|
| Issue | #25 |
| Branch | `wp/ENG-05` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-eng-05-a650, lease until 2026-11-03T15:53:50Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Write `fill()` bodies as vectored writes from the shared filler buffer, so large bodies are not copied per session, which is Gate 0 carry over item 1. Output bytes stay identical for the same seed. A bulk benchmark records a baseline in `bench/baselines/`, and the 1 MiB body workload stays within 25% of the hand written baseline on the same host.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Render parts that point into the filler, with tests that the bytes and draws match `render_into`
- [ ] 2. Vectored send path in `fps-interp`, with component tests
- [ ] 3. Criterion bulk benchmark, adapter to fps-bench/v1, and the recorded baseline
- [ ] 4. Engine page section, log entry, handoff, and pull request

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
| 2026-11-03T14:24Z | claude-eng-05-a650 | claim | none | Claimed with card 6606329427 |
