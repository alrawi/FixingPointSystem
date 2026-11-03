# Handoff CTL-02 REST layer from the OpenAPI contract

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #40 |
| Branch | `wp/CTL-02` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-ctl-02-fb93, lease until 2026-11-03T04:13:38Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Generate the server interface from contracts/openapi/fps-api-v1.yaml and add RFC 9457 problem details, read, run, and admin token scopes, and 501 stubs. A contract test calls every operation and validates each response against the OpenAPI document.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Pin oapi-codegen, generate types and the server interface, add the freshness check
- [ ] 2. Problem responses and the problem helper
- [ ] 3. Token scopes middleware
- [ ] 4. Stub server that returns 501 problems and the public constructor
- [ ] 5. Contract test with kin-openapi over every operation
- [ ] 6. Docs, log entry, make targets, CI

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
| 2026-11-03T02:43Z | claude-ctl-02-fb93 | claim | | Session start |
