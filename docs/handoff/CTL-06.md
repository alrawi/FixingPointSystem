# Handoff CTL-06 Agent manager

| Field | Value |
|---|---|
| Issue | #44 |
| Branch | `wp/CTL-06` |
| Pull request | #96 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-ctl-06-78c2, lease until 2026-11-03T13:09:24Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We track agent registration, the inventory, reservation, and heartbeat timeouts in the controller. The controller serves the Register call and the Session stream of ControllerService, and it serves the /v1/agents operations.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [x] 2. Generate Go code from agent.proto into `internal/agents/agentpb` with a freshness test
- [x] 3. Add the inventory with enrollment, reservation, release, and heartbeat timeouts
- [x] 4. Serve Register and Session and the /v1/agents operations
- [x] 5. Add the fake agent package and the three agent test
- [x] 6. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

Each step was verified with the command after it.

- Step 2. `go test ./internal/agents/agentpb/gen` regenerates `agent.pb.go` and `agent_grpc.pb.go` and finds them current.
- Steps 3 to 5. `make lint-ctl test-ctl` passes, and golangci-lint reports 0 issues. `go test -race -count=5 ./internal/agents/` passes, and `-coverpkg` over `internal/agents` and `internal/fakeagent` reports 90.0%.
- The acceptance test is `TestThreeAgents` in `controller/internal/agents/agents_test.go`. It covers registration, reservation, release, and a missed heartbeat with three fake agents.
- Step 6. `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass. The Controller page has an Agent manager section, and the log entry is `docs/wiki/log/2026-11-03T1157-ingest-ctl-06-agent-manager.md`.

## Remaining

The review. Known gaps follow.

- The controller binary does not start a gRPC server with mutual TLS yet. The configuration has no TLS or listener setting, and wiring belongs to a later package.
- The inventory lives in memory, so a controller restart forgets enrollments and reservations.
- `GET /v1/capabilities` stays unimplemented. It can read `Manager.List` when a package implements it.
- Package `fakeagent` has no tests of its own. The agents tests cover it.

## Next action

A reviewer agent reviews #96 against the Definition of Done.

## Blockers and open questions

None.
