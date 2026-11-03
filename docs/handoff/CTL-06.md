# Handoff CTL-06 Agent manager

| Field | Value |
|---|---|
| Issue | #44 |
| Branch | `wp/CTL-06` |
| Pull request | none yet |
| Status | in progress |
| Progress | 17% (1 of 6 plan steps) |
| Current holder | claude-ctl-06-78c2, lease until 2026-11-03T13:09:24Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We track agent registration, the inventory, reservation, and heartbeat timeouts in the controller. The controller serves the Register call and the Session stream of ControllerService, and it serves the /v1/agents operations.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [ ] 2. Generate Go code from agent.proto into `internal/agents/agentpb` with a freshness test
- [ ] 3. Add the inventory with enrollment, reservation, release, and heartbeat timeouts
- [ ] 4. Serve Register and Session and the /v1/agents operations
- [ ] 5. Add the fake agent package and the three agent test
- [ ] 6. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

## Remaining

Steps 2 to 6.

## Next action

Generate the protobuf code.

## Blockers and open questions
