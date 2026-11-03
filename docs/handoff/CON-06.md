# Handoff CON-06 Agent initiated control connection

| Field | Value |
|---|---|
| Issue | #76 |
| Branch | `wp/CON-06` |
| Pull request | #79 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-con-06-5e8a, lease until 2026-11-03T04:13:25Z |
| Last verified commit | see the session log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply the owner decision of 2026-11-02 that agents register with the controller. The agent dials the controller, so agents behind NAT or in clouds need no inbound port. The controller serves a `ControllerService` with `Register` and a bidirectional `Session` stream that carries heartbeats, Prepare, Start, Stop, and counter snapshots.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Rework `contracts/proto/fps/agent/v1/agent.proto` around `ControllerService` and pass `buf lint` and `buf format`.
- [x] 2. Update the state machine, error codes, and example plan notes in `contracts/README.md`.
- [x] 3. Update `tools/contracts/check_plans.py` and its tests for the new messages.
- [x] 4. Update the OpenAPI `registerAgent` description and add `contracts/changes/CON-06.md`.
- [x] 5. Create `docs/wiki/Contracts.md` and the log entry.
- [x] 6. Run the checks, push, wait for CI, and finish the pull request.

## Done and verified

1. `buf format --diff --exit-code contracts/proto` and `buf lint contracts/proto` pass with buf 1.74.0.
2. `buf breaking` against `origin/main` reports the removal of `AgentService` and the renumbered `Register` messages, which is the intended change.
3. `python3 tools/contracts/check_plans.py` reports 0 errors, and `python3 -m pytest -q tools/contracts` passes 159 tests, including 6 new protocol shape tests.
4. `python3 tools/contracts/validate.py` reports 0 errors with the OpenAPI change.
5. `make lint test` passes, including the `fps-agent` contract test that reads the proto header and the capability messages.
6. `tools/plan/plan.py check`, `render --check`, and `lint` pass with the new page `docs/wiki/Contracts.md`.

## Remaining

Owner approval. The CI job `proto` fails at its `buf breaking` step by design, and `ci.yml` belongs to OPS packages.

## Next action

Wait for the owner review of pull request #79 and apply any requested changes.

## Blockers and open questions

The package carries `needs:owner`. The questions for the owner are listed in the pull request description.

## Decisions taken in this package

1. `AgentService` is removed entirely. Every call moved onto the session stream, and the standalone mode reads a plan file without a service, so no part of it has a reason to stay.
2. The agent identity is the identity in its client certificate, and `RegisterRequest.agent_id` must equal it. The controller does not assign identifiers, so a reconnecting agent keeps its identity.
3. `Register` stays a unary call that returns a session token, and the first `Session` message presents it. Registration therefore has plain gRPC status codes, and the session check is explicit in the contract and not hidden in metadata.
4. Commands carry a `command_id` that the result echoes, and `CommandError` carries an `ErrorCode` whose numbers equal the gRPC codes, because a stream can return only one status at its end.
5. The controller sends one command at a time, except that `stop` may follow at once. This keeps agent ordering simple and still lets an operator abort a slow prepare.
6. Both sides watch one heartbeat timeout, and the controller acknowledges each heartbeat, so the agent detects a silent controller and aborts its run.
7. `PrepareCommand` carries the counter interval and the per worker option, because no separate subscription call exists any more. Snapshot sequence numbers continue across sessions.
8. The protocol version stays 0.1.0, because 0.1.0 never shipped, and the `fps-agent` contract test pins the header to its `ENGINE_API` of 0.1.0.
9. OpenAPI `registerAgent` now enrolls an identity with `id` in place of `address`, and `Agent.address` became optional. OpenAPI moved to 0.2.0 because the request body changed.
10. `Plan`, `Capabilities`, `PortInfo`, `ModuleRef`, `CounterSnapshot`, and the existing enums keep their fields and numbers, so the example plan and the Rust mirrors stay valid.

## Pitfalls

The `fps-agent` contract test in `engine/fps-agent/tests/contract.rs` requires the proto header to contain `contract version <ENGINE_API>.`, so a version bump of the proto needs a matching change in that crate.

## Verify the current state

```bash
buf lint contracts/proto
python3 tools/contracts/check_plans.py
python3 -m pytest -q tools/contracts
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-con-06-5e8a | claim | none | Claimed issue #76 and created the branch |
| 2026-11-03T03:00Z | claude-con-06-5e8a | finish | see git log | All checks pass locally and the package waits for owner review |
