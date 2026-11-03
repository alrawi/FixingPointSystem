# Handoff CTL-07 Run orchestrator

| Field | Value |
|---|---|
| Issue | #45 |
| Branch | `wp/CTL-07` |
| Pull request | #106 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-ctl-07-8293, lease until 2026-11-03T14:47:23Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We drive the run state machine from pending to a final state and contain agent faults. The controller serves POST /v1/runs, GET /v1/runs/{id}, and POST /v1/runs/{id}:stop, and it marks a run degraded when an agent fails during the run.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [x] 2. Add the state machine with its transition table and table driven tests
- [x] 3. Add the orchestrator over store, compiler, and agent manager interfaces
- [x] 4. Serve the three run operations and test them with fake agents
- [x] 5. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

Each step was verified with the command after it.

- Step 2. `go test -race ./internal/runs/ -run 'Transition|Final'` checks all 49 pairs of states against the allowed table.
- Steps 3 and 4. `make lint-ctl test-ctl` passes, and golangci-lint reports 0 issues. `go test -race -count=30 ./internal/runs/` passes, and `-cover` reports about 91%.
- The acceptance test for faults is `TestFailingAgentEndsRunByCriteria` in `controller/internal/runs/orchestrator_test.go`. One of two fake agents fails or crashes under way, the run turns degraded, and the session failure ratio against the criterion of 10% completes or fails the run.
- Step 5. `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass. The Controller page has a Run orchestrator section, and the log entry is `docs/wiki/log/2026-11-03T1317-ingest-ctl-07-run-orchestrator.md`.

## Remaining

The review. Known gaps follow.

- The controller binary does not build the orchestrator yet. Wiring needs the store, the agent manager with `Observer` set to `Orchestrator.Observe`, and a `runs.Catalog` backed by the registry, which no package provides yet.
- Runs live in memory, so a controller restart forgets them. `GET /v1/runs` stays unimplemented.
- An agent that reconnects during a run stays failed for that run.
- The default evaluator checks only metrics present in the summed counters, such as `sessions.failed_ratio`. CTL-09 owns the full criteria.
- A failure reason lives in `Run.Reason` and the logs, because the Run schema has no field for it.

## Next action

A reviewer agent reviews #106 against the Definition of Done.

## Blockers and open questions

None.

## Decisions taken in this package

- Create reserves the agents and compiles the plans before it answers 202, so configuration and agent errors return a problem at once instead of a failed run whose reason the Run schema cannot carry.
- Only stopping may complete. Agents that finish on their own still pass through stopping, where the orchestrator evaluates the criteria.
- The criteria decide every stopped run, degraded or not. A failing verdict ends the run as failed.
- The sessions that a failed agent attempted without success or failure count as failed, because the crash ended them.
- Counters merge per metric across snapshots, because the fake agent and the contract allow a final snapshot without counters.
- A request without agents takes the operator reserved agents and returns their reservation to the operator when the run ends.

## Pitfalls

- An abort must run outside the run context, because a forced stop cancels that context.
- An agent that answered prepare with an error holds no plan, so it gets no abort.
- Before a test stops heartbeats, it sends one acknowledged heartbeat, so the heartbeats of the start do not land after the clock advance.

## Verify the current state

```bash
cd controller && GOWORK=off go test -race ./internal/runs/...
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T12:56Z | claude-ctl-07-8293 | claim | none | Claimed from status:ready |
| 2026-11-03T13:15Z | claude-ctl-07-8293 | checkpoint | see git log | State machine, orchestrator, REST layer, and tests |
| 2026-11-03T13:20Z | claude-ctl-07-8293 | finish | see git log | Ready for review |
