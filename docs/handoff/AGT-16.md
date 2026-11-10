# Handoff AGT-16 Multi flow scenarios and digested extension bytes in the agent

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #350 |
| Branch | `wp/AGT-16` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-eb8bb0, lease until 2026-11-10T14:16:43Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Run multi flow scenarios in the agent (ENG-23 worker), load extension.wasm from the digested bytes, register http2, use with_client_factory per session, set FlowRef::port, and compile with Scenario::parse_instance.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. modules.rs: digest-bound extension bytes, parse_instance, http2 registry entry
- [ ] 2. assemble.rs: multi flow, UDP refusal, FlowRef::port
- [ ] 3. tls.rs: per session client connectors, profile names
- [ ] 4. tests, docs, log entry, PR

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
| 2026-11-10T13:48Z | claude-eb8bb0 | claim | | Claimed #350, branch from origin/main 14894c3 |
