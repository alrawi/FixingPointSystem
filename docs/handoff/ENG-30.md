# Handoff ENG-30 Count a scenario session once, with the bytes of its spawned flows

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #535 |
| Branch | `wp/ENG-30` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-d3e952, lease until 2026-11-13T06:39:19Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement CON-31 in `engine/fps-runtime`: a session is one run of the entry flow of a scenario instance, every other flow adds its bytes and its failure to it, and `sessions.concurrent` and `sessions.duration` span the instance. Update the tests that count sessions of multi flow scenarios, and Engine.md.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Engine: instance.rs and worker.rs record one session per instance
- [ ] 2. fps-runtime tests (instances.rs, datagrams.rs, tls_sessions.rs)
- [ ] 3. fps-agent tests (first_party.rs, controller_e2e.sh)
- [ ] 4. Engine.md and the log entry with the Shared page notes
- [ ] 5. Full checks, PR, CI, review

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
| 2026-11-13T05:10Z | claude-d3e952 | claim | - | claimed #535, branch from origin/main 61ff082 |
