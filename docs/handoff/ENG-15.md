# Handoff ENG-15 New failure reasons and side label in the interpreter and runtime

| Field | Value |
|---|---|
| Issue | #217 |
| Branch | `wp/ENG-15` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-e15a7c, lease until 2026-11-07T18:52:08Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Report the new failure reasons (`datagram_rejected`, `template_error`, `extension_error`) from `FailureKind::reason`, and register the runtime session series, including `sessions.concurrent` and `sessions.duration`, with the `side` label through `fps_stats::SessionCounters`.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. `FailureKind::reason` and its tests in `engine/fps-interp/`.
- [ ] 2. `SessionCounters` gains concurrent and duration; `fps-runtime` uses it; snapshot tests for both sides.
- [ ] 3. Engine wiki page, log entry, checks, pull request.

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Start step 1.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
cargo test -p fps-interp -p fps-stats -p fps-runtime
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-07T17:23:00Z | claude-e15a7c | start | none | claimed #217 |
