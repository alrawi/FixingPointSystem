# Handoff AGT-02 Agent gRPC service

| Field | Value |
|---|---|
| Issue | #37 |
| Branch | `wp/AGT-02` |
| Pull request | see the card |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-agt-02-641d |
| Last verified commit | see branch |

## Goal

Implement the agent side of the control protocol. The agent dials the controller, calls `Register`, opens the `Session` stream, serves `prepare`, `start`, and `stop` with the state machine of `contracts/README.md`, sends heartbeats, results, and counter snapshots, and dials again with exponential backoff after a lost connection.

## Plan

- [x] 1. Generated proto types with tonic and protox in a build script
- [x] 2. A controllable run handle over the standalone run path, fed by a proto `Plan`
- [x] 3. The session loop with the agent state machine and command results
- [x] 4. Heartbeats, the heartbeat timeout, and reconnection with backoff
- [x] 5. A test with an in process fake controller that drives prepare, start, and stop
- [x] 6. Wiki section, log entry, and finish

## Done and verified

| Step | Verification | Result |
|---|---|---|
| 1 Generated types (`build.rs`, `src/pb.rs`) | `cargo build -p fps-agent` | protox compiles agent.proto without protoc |
| 2 Run handle (`src/run.rs`, `src/convert.rs`) | `cargo test -p fps-agent --test standalone`, `cargo test -p fps-agent --lib convert` | Standalone tests unchanged and green, example plan round trips through the generated types |
| 3 and 4 Session (`src/session.rs`, `src/main.rs`) | `cargo test -p fps-agent --test session` | 4 tests pass in about 4.5 s, three runs in a row |
| 5 Fake controller and real controller | `tests/session.rs`, `tests/controller_e2e.sh` | Real fps-controller drove a loopback run to completed with 148 succeeded sessions and a passed verdict |
| 6 Wiki and log | `python3 tools/plan/plan.py lint` | ok |

`make lint-rust lint-shell lint-deny test-agt` passes.

## Remaining

- `per_worker` snapshots. The agent logs a warning and sends aggregate snapshots.
- Ports are not bound during `prepare`. Workers bind when the run starts.
- Mutual TLS waits for CTL-15.
- `tests/controller_e2e.sh` runs in no workflow yet, because `mk/agt.mk` and `agt.yml` already exist and sit outside this package. The log entry asks for a target.

## Next action

None. A reviewer checks the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- protox and tonic-prost-build generate the stubs in `build.rs`, so CI needs no protoc.
- Both client and server stubs are generated, so tests can serve a fake controller from the crate.
- The proto `Plan` converts into the standalone plan type, and snapshots convert through the proto3 JSON mapping of fps-stats, so both modes share checks and encodings.
- Error code mapping. Plan rule errors and scenario errors are INVALID_ARGUMENT, module digest, version, missing, or unreadable modules are FAILED_PRECONDITION, more workers than cores is RESOURCE_EXHAUSTED, a backend or flow kind the run path lacks is UNIMPLEMENTED, worker failures are INTERNAL and move to FAILED.
- On the heartbeat timeout the agent releases the plan from any state and moves to IDLE.
- Backoff 200 ms doubling to 10 s. A session that opened resets it, except one that ended with ABORTED.
- `stop` with STOP_MODE_RAMP_DOWN moves to STOPPING, an abort stays RUNNING until STOPPED. A second stop during a drain can shorten it.
- Logs use `tracing` with a JSON `tracing-subscriber` in the binary.

## Pitfalls

- The controller checks port locations against the agent capabilities, and capability detection leaves out loopback. The end to end script therefore names the first detected interface.
- The REST API has no `GET /v1/agents/{id}`. Use the list.
- Proto `params` numbers are doubles. The converter turns integral values into JSON integers, as plan files write them.

## Verify the current state

```bash
export CARGO_TARGET_DIR=/path/to/scratch/target
cargo test -p fps-agent                      # 43 tests pass
(cd controller && GOWORK=off go build -o /tmp/fps-controller ./cmd/fps-controller)
cargo build -p fps-agent
engine/fps-agent/tests/controller_e2e.sh /tmp/fps-controller "$CARGO_TARGET_DIR/debug/fps-agent"
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T16:09Z | claude-agt-02-641d | claim | none | Fresh branch from origin/main |
| 2026-11-03T16:20Z | claude-agt-02-641d | checkpoint | see branch | Generated types, run handle, session client |
| 2026-11-03T16:24Z | claude-agt-02-641d | checkpoint | see branch | Fake controller tests |
| 2026-11-03T16:29Z | claude-agt-02-641d | finish | see branch | Real controller script, wiki section, log entry |
