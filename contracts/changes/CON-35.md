# CON-35 The agent reports the backend that ran and the bound XDP mode

Contract. Agent control protocol `proto/fps/agent/v1/agent.proto` and controller OpenAPI document `contracts/openapi/fps-api-v1.yaml`. The change is issue #663. The vetting agent decides it under decision D13, as an additive contract change, and the pull request body carries the short rationale that it vets.

Problem. `Register` comes before any plan, so `PortInfo.xdp_driver_mode` carries only the driver table hint, and the mode that each socket really bound in (AGT-24) reaches only the agent log. After the flip of TRN-31 an unforced agent may also run a plan on the kernel backend, because its AF_XDP datapath refuses the plan (AGT-29, decision record `docs/wiki/log/2026-11-18T1900-decision-trn-31-waits.md`). `Capabilities.selected_backend` then names the backend that the agent would select, and a benchmark that reads it can measure the wrong backend unseen.

## Rationale: the fields, the message, and why there

Two fields on `Heartbeat`, and one new message.

| Field | Number | Meaning |
|---|---|---|
| `Heartbeat.run_backend` | 7 | Of type `Backend`. The backend that started the loaded plan. `BACKEND_UNSPECIFIED` until the agent has started the plan, and for an agent of 0.2.6 or earlier |
| `Heartbeat.run_ports` | 8 | Repeated `RunPort`. One entry for each `PortBinding` of the plan, in plan order, empty on the same terms as `run_backend` |
| `message RunPort` | | `name` (1) is `PortBinding.name`, `interface_name` (2) is `PortBinding.interface_name` and so `PortInfo.name`, and `xdp_mode` (3) is an `XdpMode` |

`RunPort.xdp_mode` is the mode that the sockets really bound in, and `XDP_MODE_UNSPECIFIED` when `run_backend` is not `BACKEND_AF_XDP`. A port with queues in more than one mode reports the slowest of them, in the order `GENERIC`, `NATIVE`, `ZERO_COPY`, which is the rule of CON-32 for `PortInfo.xdp_driver_mode`. It reuses the enum `XdpMode`, so its numbers and its table of TRN-11 bind modes need no second definition. It differs in meaning from `PortInfo.xdp_driver_mode`, which stays a driver table hint and is the only value that exists at `Register`: the new field is what the run measured, and a reader may use it to label a result.

Why the heartbeat and not the start result.

- The start result is sent once, as the answer to one command. When the stream breaks after the start and the agent registers again within the heartbeat timeout, the run continues (contract README, state machines), and the controller cannot learn from a result that it never receives again. The agent sends a heartbeat per interval on every session, so the controller learns the backend again after a lost connection, and the controller only has to keep the latest values.
- The heartbeat already carries the per run state of the agent (`state`, `plan_id`, `phase`, `message`), and it is sent at once after every state change, so the values arrive with the change to `RUNNING`. They stay in the heartbeats of `STOPPED` and `FAILED` until the plan is released, so a run that ends at once is still reported.
- A new field on `StartResult` would need the agent to know the bound modes when it answers `Start`, and it would put the same data in a second place if a later package also wanted it for the heartbeat.

A field on `Heartbeat` costs a few bytes per interval, as one `RunPort` is two short strings and an enum for each port of the plan.

The REST mirror is `Run.agents`, an optional array of the new `AgentRun` (`agent`, `backend`, optional `selected_backend`, optional `ports` with `name`, `interface_name`, and `xdp_mode`). `selected_backend` is the backend that the inventory listed when the run started, so a reader sees the mismatch without a second request: a `backend` that differs from it is a run that did not measure what the inventory promised. The controller (CTL-55) fills the array from the heartbeats and `Agent.backend`, and the run report carries it through `RunReport.run`.

## Versions

- The agent protocol moves from 0.2.6 to 0.2.7. The change adds two fields and a message and no command, so it takes a patch number for the reason that CON-11, CON-13, CON-15, CON-24, CON-32, and CON-34 gave: the controller admits agents by the major and minor part of `Capabilities.engine_api`, and a 0.3.0 would make it refuse every agent until its list changes. The header comment of the proto file, `buf.yaml`, `contracts/README.md`, and `docs/wiki/Contracts.md` name the new version. The Rust agent reports `ENGINE_API` 0.2.7 once it sends the fields (AGT-29), because `engine/fps-agent/tests/contract.rs` ties `ENGINE_API` to the header comment.
- OpenAPI moves from 0.13.0 to 0.14.0. The change adds an optional response property and a schema, which a reader sees in the document, so it takes a minor number, as CON-32 did. `api.APIVersion` follows, because `capabilities_test.go` compares it with `info.version`. `openapi/fps-metrics.yaml` stays at 0.3.0.
- The metric catalog, the scenario contract, the test configuration, and the module manifest schema stay as they are.

Field numbers 7 and 8 of `Heartbeat` were free: the message ends at `message = 6`, and the file declares no `reserved` range. Nothing is renumbered.

## Compatibility

The change is additive.

- `buf breaking` with the `FILE` rules reports no finding against `main`, because the change only adds fields with new numbers and a message (CI checks it, because `buf` is not installed in the sandbox of this package). The proto file compiles with `grpc_tools.protoc`, and a comparison of the compiled descriptors of `main` and of this change shows one new message, two new fields of `Heartbeat`, and no field or enum value that was removed, renumbered, or retyped.
- Wire level. A controller of 0.2.6 skips the two fields of an agent of 0.2.7 and sees what it saw before. An agent of 0.2.6 sends neither, so a controller of 0.2.7 reads `BACKEND_UNSPECIFIED` and an empty list as "not reported", and then shows no `agents` entry for that agent, as the property says. No command or plan changes.
- REST. The change adds one optional property of `Run` and one schema, and removes or changes no type or enum value, so the `openapi-breaking` workflow (oasdiff) finds nothing breaking (CI checks it, because `oasdiff` is not installed in the sandbox of this package). A client that ignores the property sees no difference. A controller older than 0.14.0 does not send it.
- Generated code. A literal `agentpb.Heartbeat{...}` with keyed fields still compiles. A Rust literal `pb::Heartbeat { ... }` without `..Default::default()` does not compile once `build.rs` regenerates `pb`: `engine/fps-agent/src/session.rs` (`fn heartbeat`) builds it with the six fields.

## Consumers

The order is the order of the work. Nothing before step 1 compiles against the new fields.

| Step | Consumer | What it does with the change |
|---|---|---|
| 1 | Coordinator, regeneration | Runs the workflow `regen-agentpb` (Go bindings of the proto file), then in `controller/` runs `go generate ./internal/api` and sets `APIVersion` to `0.14.0` in `internal/api/capabilities.go` (`capabilities_test.go` compares it with `info.version`). The SDK (`sdk/python/fixingpoint/_generated.py`) and the UI (`ui/src/api/schema.d.ts`) are regenerated in this change, with `python3 sdk/python/scripts/generate_client.py` and the generator of `ui/scripts/gen-api.mjs` |
| 1 | Agent compile fix and version | `engine/fps-agent/src/session.rs` builds `pb::Heartbeat` with the six old fields, so the crate does not compile until it names the two new ones (`..Default::default()` is enough, and AGT-29 fills them). `ENGINE_API` in `src/capabilities.rs` and `tests/detect.rs` become 0.2.7 with AGT-29, or with the coordinator's commit, because `tests/contract.rs::engine_api_matches_the_proto_contract_version` compares `ENGINE_API` with the proto header |
| 2 | Agent, `engine/fps-agent` (AGT-29) | Keeps the backend that it started and the bound mode of each port of the plan in the state that `fn heartbeat` reads, fills `run_backend` and `run_ports` from the first heartbeat after `Start` until the plan is released, and sends a heartbeat at once after the start. An unforced agent that runs a refused plan on the kernel backend reports `BACKEND_KERNEL` with `XDP_MODE_UNSPECIFIED` for each port. `RunPort.xdp_mode` is the mode that `BoundPort` reads from the sockets (AGT-24), by the table of CON-32 |
| 3 | Controller, store and API (CTL-55) | Keeps the latest `run_backend` and `run_ports` of each agent of a run, with `Agent.backend` at the start of the run as `selected_backend`, and serves `Run.agents` in `getRun`, `listRuns`, and the run report. A run whose `backend` differs from `selected_backend` is marked in the report. The fake agent (`controller/internal/fakeagent`) may fill the fields to test the pass through |
| | UI, CLI, SDK | No component reads `Run.agents` today. The generated types of the UI and the SDK change only |

## Verification

`python3 tools/contracts/validate.py` passes, `python3 tools/contracts/check_plans.py` passes, `python3 -m pytest tools/contracts` passes (377 passed, 1 skipped), `python3 -m pytest sdk/python` passes, the proto file compiles with `grpc_tools.protoc`, and `python3 tools/plan/plan.py check` passes. The new tests in `tools/contracts/test_plans.py` check the field numbers and types, a round trip of a kernel fallback and of an AF_XDP run, the empty reading of a heartbeat of 0.2.6, and the version in the proto header, `buf.yaml`, and the README. The test in `tools/contracts/test_contracts.py` checks the REST schema. The pull request CI checks `buf`, `oasdiff`, and the workspace tests. The jobs that read `APIVersion`, `ENGINE_API`, and the generated Go code fail on this branch until the consumer steps above land.
