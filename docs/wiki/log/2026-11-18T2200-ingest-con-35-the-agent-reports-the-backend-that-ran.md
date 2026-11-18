## [2026-11-18] ingest | CON-35 The agent reports the backend that ran and the bound XDP mode

CON-35 (issue #663) adds `Heartbeat.run_backend` (7), `Heartbeat.run_ports` (8), and the message `RunPort` to the agent protocol, which moves from 0.2.6 to 0.2.7, and mirrors them in the REST `Run` as `agents` (schema `AgentRun`), which moves the controller API from 0.13.0 to 0.14.0. After the flip of TRN-31 an unforced agent may run a plan on the kernel backend (AGT-29), and `Register` comes before any plan, so neither `Capabilities.selected_backend` nor `PortInfo.xdp_driver_mode` says what measured a run. The fields carry the backend that ran the plan and the bound XDP mode of each port, so a benchmark never measures the wrong backend unseen. AGT-29 fills them and CTL-55 stores and shows them.

Decisions.

- The fields ride on the heartbeat and not on `StartResult`. The start result is sent once, so a controller that lost the stream during a run and resumed it from the reported state would never learn them. The heartbeat repeats on every session, carries the other per run state, and is sent at once after every state change.
- `RunPort.xdp_mode` reuses `XdpMode` and the slowest queue rule of CON-32. It means the mode that the sockets really bound in, where `PortInfo.xdp_driver_mode` stays a driver table hint. `RunPort` names the port by `PortBinding.name` and by the interface, so a reader joins it to the plan and to `PortInfo.name`.
- `AgentRun.selected_backend` is the backend that the inventory listed when the run started, so a report shows the mismatch that CTL-55 marks without a second request.
- No reason for a fallback travels in the protocol. The agent log (AGT-29) says why a plan ran on the kernel backend.

Findings.

- Any field added to `Heartbeat` stops `engine/fps-agent` from compiling, because `fn heartbeat` in `src/session.rs` builds `pb::Heartbeat` with the six old fields and no `..Default::default()`. `StartResult` would have stopped it at three more places. The compile fix and the `ENGINE_API` bump to 0.2.7 are listed in the Consumers table of `contracts/changes/CON-35.md`, and this package did not make them, because it changes no agent code.
- The Go bindings (`regen-agentpb`), `api.gen.go`, and `APIVersion` need the coordinator, because Go modules cannot be downloaded in the sandbox. The SDK and the UI types were regenerated here.

### Shared page notes

- Implementation Plan and Work Packages: AGT-29 and CTL-55 can start once this package merges. The coordinator lands the consumer commit in the pull request (`regen-agentpb`, `go generate ./internal/api`, `APIVersion` 0.14.0, the `pb::Heartbeat` literal and `ENGINE_API` of the agent) as for CON-32 and CON-34.
- Agent page (AGT workstream): after AGT-29, say that the heartbeat carries `run_backend` and `run_ports` from the start until the plan is released.
- Module Development: no change, because no module field, scenario action, or template function changed.
