# CON-06 Agent initiated control connection

Contract. Agent control protocol in `proto/fps/agent/v1/agent.proto` and the controller OpenAPI document.

Versions. The agent control protocol stays at 0.1.0, which never shipped and sits under Unreleased in `CHANGELOG.md`, so this change revises that draft in place. The Unreleased entry for CON-01 should describe `ControllerService` in place of `AgentService` when the coordinator folds this file. OpenAPI moves from 0.1.1 to 0.2.0, because its request body changed.

- Agents dial the controller and register themselves, as the owner decided on 2026-11-02. The controller serves `ControllerService` with the unary `Register` and the bidirectional `Session` stream. The agent serves no gRPC service.
- `AgentService` is removed with all its request and response messages. No part of it is kept, because every call moved onto the session stream and the standalone mode reads a plan file without any service.
- On the session stream the agent sends `open`, `heartbeat`, `result`, and `snapshot`, and the controller sends `heartbeat_ack`, `prepare`, `start`, and `stop`. Each command carries a `command_id` that its `CommandResult` echoes.
- `PrepareCommand` carries the counter interval and the per worker option that `StreamCountersRequest` carried before. Snapshot sequence numbers continue across sessions.
- The new enum `ErrorCode` reports command failures with the numbers of the gRPC status codes, and `ERROR_CODE_INTERNAL` is new. `Register` and `Session` end with gRPC status codes, which `contracts/README.md` lists.
- Both sides watch one heartbeat timeout. An agent that hears nothing from the controller aborts its run, and an agent that reconnects within the timeout resumes it.
- `Plan`, `Capabilities`, `PortInfo`, `ModuleRef`, `CounterSnapshot`, and every earlier enum are unchanged, so the example plan and the Rust mirrors in `engine/fps-agent` stay valid.
- OpenAPI `registerAgent` now enrolls an agent identity ahead of time. Its request takes `id` in place of `address`, and `Agent.address` is optional and holds the peer address of the open session.
- `buf breaking` against the base branch reports the removal of `AgentService` and the renumbered `Register` messages. The change is intentional, since protocol 0.1.0 never shipped and no code generates from it yet.
