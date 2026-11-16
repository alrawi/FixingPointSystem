# Handoff CTL-16 Multi agent runs

| Field | Value |
|---|---|
| Issue | #567 |
| Branch | `wp/CTL-16` |
| Pull request | #582 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-f9c390, lease until 2026-11-17T00:23:07Z |
| Last verified commit | f9becbb (CI green on PR #582, controller test job ran `internal/multiagent`) |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Split plans across several agents, check clock synchronization before the start, and aggregate the results of the agents. Acceptance: a run with three fake agents aggregates counters exactly and refuses to start with a clock offset above 10 ms.

## Plan

- [x] 1. Package skeleton, errors, and the clock offset check (`clock.go`)
- [x] 2. Exact counter aggregation across agents (`aggregate.go`)
- [x] 3. Plan splitting wrapper (`split.go`)
- [x] 4. The multi agent run: check, reserve, prepare, start at a common instant, stop, aggregate (`run.go`)
- [x] 5. Tests with three fake agents, wiki page and log entry, PR

## Done and verified

- Steps 1 to 4: `clock.go`, `aggregate.go`, `split.go`, `run.go`, and `doc.go` in `controller/internal/multiagent/`. `gofmt -l controller/internal/multiagent` prints nothing. The Go build cannot run in this sandbox (no module proxy), so the pull request CI is the build and test loop.
- Tests written: `clock_test.go`, `aggregate_test.go`, `split_test.go`, `run_test.go`. `run_test.go` uses the real agent manager, the real compiler, and three fake agents over in memory gRPC.
- `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Nothing in this package. Follow up outside it: record `agent_time` in `internal/agents` and wire a `ClockSource` (see Decisions).

## Next action

Coordinator review and merge.

## Blockers and open questions

none

## Decisions taken in this package

- `ClockSource` is an interface of this package. The agent manager discards `Heartbeat.agent_time`, and `internal/agents` and `internal/fakeagent` are outside this package's paths, so the controller has no production source yet. The contract (`agent.proto`) has the fields that the estimate needs, so no contract change is requested. Follow up for the owner of CTL-06: record `agent_time` minus the receive time per agent and expose it.
- The 10 ms limit applies to the absolute offset of each agent against the controller, as the acceptance reads. An offset of exactly 10 ms passes.
- A missing measurement refuses the start (fail closed).
- The check runs before reserve, prepare, or any command, so a refusal leaves the agents untouched. The test asserts zero received commands.
- Counters sum in uint64 with overflow detection, never through a float.
- The start instant is controller time after the last prepare plus a lead of 2 s, converted per agent with its offset.
- No wiring into `internal/wiring` or the REST layer: the catalog path list is only `controller/internal/multiagent/`.

## Pitfalls

Go modules cannot be downloaded in this sandbox, so Go code is checked with `gofmt -l` locally and built by the CI of the pull request.

## Verify the current state

```bash
gofmt -l controller/internal/multiagent/
```

Expected result: no output.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16T22:53Z | claude-f9c390 | claim | none | Claimed #567 with card 6931811598 |
| 2026-11-16T23:30Z | claude-f9c390 | checkpoint | see branch | Package, tests, wiki page, and log entry written |
| 2026-11-16T23:25Z | claude-f9c390 | finish | f9becbb | PR #582 open, all CI checks green, set to ready for review |
