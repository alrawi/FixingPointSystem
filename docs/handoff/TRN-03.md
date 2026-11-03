# Handoff TRN-03 Operating system profiles for the kernel backend

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #33 |
| Branch | `wp/TRN-03` |
| Pull request | #0 |
| Status | in progress |
| Progress | 20% (1 of 5 plan steps) |
| Current holder | claude-trn-03-c222, lease until 2026-11-03T13:08:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the OS profiles windows11, linux6, and macos14 as data files in `engine/fps-backend-kernel`, apply their TTL, MSS, and window clamp per socket, and document in a limits table which options the kernel backend cannot set per socket. A capture from the QA-01 namespace harness shows the configured TTL and MSS for each profile, and the workflow `trn.yml` runs it with sudo.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim, branch, handoff, and pull request
- [ ] 2. Profile data files, the profile type, and per socket options with unit tests
- [ ] 3. Probe example and loopback test
- [ ] 4. Namespace capture test, `mk/trn.mk`, and `trn.yml`
- [ ] 5. Transport page section, log entry, checks, CI, and review

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
| 2026-11-03T11:38Z | claude-trn-03-c222 | claim | | card 6602800166 |
