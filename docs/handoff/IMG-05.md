# Handoff IMG-05 Container images

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #201 |
| Branch | `wp/IMG-05` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-4643a2, lease until 2026-11-09T21:25:35Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Build controller and agent container images (Dockerfiles with non root users) and a compose file for an all in one lab. Acceptance: the compose lab runs a short test between two agent containers. The workflow `.github/workflows/containers.yml` builds the images and runs the lab on a GitHub hosted runner.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Dockerfiles for controller and agent
- [ ] 2. Compose lab, init scripts, and the test
- [ ] 3. Workflow containers.yml
- [ ] 4. Docs (Image.md Containers section), log entry
- [ ] 5. PR and green CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Write the Dockerfiles.

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
| 2026-11-09T21:00Z | claude-4643a2 | claim | - | claimed #201 |
