# Handoff CTL-04 Module registry

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #42 |
| Branch | `wp/CTL-04` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-ctl-04-70e2, lease until 2026-11-03T13:08:08Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

List, install, inspect, and delete module packages with manifest validation and content digests. The registry keeps a local directory with an index file and serves the /v1/modules operations.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Read the manifest schema, examples, digest rule, and generated API
- [ ] 2. Registry store with an index file, the tree digest, and manifest validation
- [ ] 3. License allow list and the lookup interface for CTL-03
- [ ] 4. The /v1/modules handlers that wrap api.Unimplemented, with tests
- [ ] 5. Docs, log entry, lint, test, CI

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
| 2026-11-03T11:38Z | claude-ctl-04-70e2 | claim | none | session start |
