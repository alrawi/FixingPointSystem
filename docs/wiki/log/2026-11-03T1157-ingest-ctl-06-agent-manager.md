## [2026-11-03] ingest | CTL-06 Agent manager

We added the agent manager `controller/internal/agents`, the generated protocol package `controller/internal/agents/agentpb`, and the fake agents `controller/internal/fakeagent`, and an Agent manager section at the end of the [Controller](Controller.md) page. The manager serves `Register` and `Session` of `ControllerService`, keeps the inventory with enrollment, reservation, release, and heartbeat timeouts, and serves the `/v1/agents` operations as a type that embeds `api.Unimplemented`. The generator compiles `agent.proto` with protocompile and runs the pinned plugins through `go tool`, and a test keeps the generated files current. `TestThreeAgents` covers registration, reservation, release, and a missed heartbeat with three fake agents and a fake clock. The module `controller/go.mod` gained `google.golang.org/grpc`, `google.golang.org/protobuf`, and `github.com/bufbuild/protocompile`, two `tool` lines for the plugins, and newer indirect `golang.org/x` modules. The evidence is `make lint-ctl test-ctl` and `docs/handoff/CTL-06.md` on the branch `wp/CTL-06`.

## Shared page notes

The coordinator may apply four notes.

1. The Layout table of [Controller](Controller.md) can gain the rows `internal/agents/` with the purpose "Agent manager, Register and Session, inventory, and /v1/agents", and `internal/fakeagent/` with the purpose "Fake agents for controller tests".
2. [Testing](Testing.md) can name `controller/internal/fakeagent` as the fake for agent neighbors in controller component tests.
3. CTL-05 can move from its hand written plan types to `agentpb.Plan` in a follow up, since the controller now generates protobuf code.
4. [Home](Home.md) can mark CTL-06 as ready for review.
