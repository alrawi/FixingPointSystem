# Handoff RLM-03 Module realism CI job

| Field | Value |
|---|---|
| Issue | #53 |
| Branch | `wp/RLM-03` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-rlm-03-ae11, lease until 2026-11-03T17:38:24Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Run every changed module in the namespace harness, capture its traffic, and fail the pull request when a realism check fails. The workflow detects the changed module directories, scores each fresh capture with nDPI, tshark, and Zeek, and uploads the realism reports as a workflow artifact. A pull request that breaks the http1 request line fails the job.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Module selection from changed paths in `realism/modules.py` with unit tests
- [ ] 2. Per module capture and score script under `realism/ci/`
- [ ] 3. Workflow `.github/workflows/realism.yml` with the detect and module jobs and the report artifact
- [ ] 4. Negative control that a broken http1 request line fails the score
- [ ] 5. Component page, log entry, CI green, and review

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
| 2026-11-03T16:10Z | claude-rlm-03-ae11 | claim | | Branch created from origin/main 426fcb7 |
