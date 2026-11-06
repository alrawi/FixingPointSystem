## [2026-11-06] ingest | CON-05 agent control protocol v0.2

The agent control protocol moves to 0.2.0. `SessionResponse` carries the new commands `update_rates`, `start_capture`, and `stop_capture`, and `CommandResult` carries their results. `ModuleRef` gains the fields `signature` and `signature_key_id`, and its field number 6 stays free for the `settings` field of CON-11. `buf breaking` reports no finding against 0.1.0, and a control run that moved a field number showed that the check detects breaks. We described the additions in [Contracts](../Contracts.md#version-020-additions), and `contracts/changes/CON-05.md` states the compatibility.

### Shared page notes

- [Contracts](../Contracts.md): the table of contract files in `contracts/README.md` still names protocol 0.1.0, and that file lies outside the paths of CON-05. The coordinator should update the row when it folds the entry.
- [Testing](../Testing.md) and the catalog: nothing changes, but the shape test `tools/contracts/test_plans.py` now lists the new command arms and checks the `ModuleRef` field numbers.
- Generated and mirrored code that must follow the proto file, owned by other packages: `controller/internal/agents/agentpb/agent.pb.go` and `agent_grpc.pb.go` (run `go generate .` in that directory, and the test `TestGeneratedCodeIsCurrent` fails until then), `engine/fps-agent/src/proto.rs` and `tests/contract.rs` (the `ModuleRef` mirror), `engine/fps-agent/src/convert.rs` (two `pb::ModuleRef` literals), and `engine/fps-agent/src/capabilities.rs` (`ENGINE_API` and the header test).
