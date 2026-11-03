# Handoff CON-06 Agent initiated control connection

| Field | Value |
|---|---|
| Issue | #76 |
| Branch | `wp/CON-06` |
| Pull request | see the session log |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-con-06-5e8a, lease until 2026-11-03T04:13:25Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply the owner decision of 2026-11-02 that agents register with the controller. The agent dials the controller, so agents behind NAT or in clouds need no inbound port. The controller serves a `ControllerService` with `Register` and a bidirectional `Session` stream that carries heartbeats, Prepare, Start, Stop, and counter snapshots.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Rework `contracts/proto/fps/agent/v1/agent.proto` around `ControllerService` and pass `buf lint` and `buf format`.
- [ ] 2. Update the state machine, error codes, and example plan notes in `contracts/README.md`.
- [ ] 3. Update `tools/contracts/check_plans.py` and its tests for the new messages.
- [ ] 4. Update the OpenAPI `registerAgent` description and add `contracts/changes/CON-06.md`.
- [ ] 5. Create `docs/wiki/Contracts.md` and the log entry.
- [ ] 6. Run the checks, push, wait for CI, and finish the pull request.

## Done and verified

None yet.

## Remaining

Every plan step.

## Next action

Rework the proto file around `ControllerService`.

## Blockers and open questions

The package carries `needs:owner`. The owner approves the pull request.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

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
