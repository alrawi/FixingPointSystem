# CON-32 Kernel release, selected backend, and XDP mode in the agent capabilities

Contract. Agent control protocol `proto/fps/agent/v1/agent.proto` and controller OpenAPI document `contracts/openapi/fps-api-v1.yaml`. The change is issue #74 in part (issue #576). The vetting agent approved it on 2026-11-16 under decision D13, as an additive contract change. The NUMA fields of issue #74 stay open, because v1 has no consumer of them.

Problem. The REST `Agent.backend` takes the first entry of `Capabilities.backends`, which is always `kernel`, so a controller shows `kernel` for an agent that selected AF_XDP. The agent selects a backend at start (TRN-14), and neither the protocol nor the REST API names the one it selected. An operator who compares two runs also cannot see the kernel release that a result came from, or whether AF_XDP ran in the driver with zero copy, in the driver with copy mode, or in the generic hook. The three differ by a large factor in the rate that one port reaches, and a driver table in a report needs the mode.

## Versions

- The agent protocol moves from 0.2.4 to 0.2.5. The change adds fields and an enum and no command, so it takes a patch number for the reason that CON-11, CON-13, CON-15, and CON-24 gave: the controller admits agents by the major and minor part of `Capabilities.engine_api`, and a 0.3.0 would make it refuse every agent until its list changes. The header comment of the proto file, `buf.yaml`, `contracts/README.md`, and `docs/wiki/Contracts.md` name the new version. The Rust agent reports `ENGINE_API` 0.2.5 once it sends the fields, because `engine/fps-agent/tests/contract.rs` ties `ENGINE_API` to the header comment.
- OpenAPI moves from 0.12.0 to 0.13.0. The change adds optional response properties, which a reader sees in the document, so it takes a minor number, as CON-12, CON-14, CON-15, and CON-24 did. `api.APIVersion` follows, because `capabilities_test.go` compares it with `info.version`. `openapi/fps-metrics.yaml` stays at 0.3.0.
- The metric catalog, the scenario contract, the test configuration, and the module manifest schema stay as they are.

## Protocol changes

`Capabilities` gains two fields and `PortInfo` gains one. The enum `XdpMode` is new.

| Field | Number | Meaning |
|---|---|---|
| `Capabilities.kernel_release` | 9 | The kernel release of the host, as `uname -r` prints it, such as `6.8.0-45-generic`. Empty for an agent of 0.2.4 or earlier, and when the agent cannot read it |
| `Capabilities.selected_backend` | 10 | The backend that the agent selected and started, one entry of `backends`. `BACKEND_UNSPECIFIED` for an agent of 0.2.4 or earlier, which always used `BACKEND_KERNEL` |
| `PortInfo.xdp_driver_mode` | 6 | The XDP mode of the port, of type `XdpMode`. `XDP_MODE_UNSPECIFIED` unless the selected backend is `BACKEND_AF_XDP` and the agent bound a queue of the port |

`XdpMode` is documented as a driver table hint. It says what rate a driver may reach and changes no command, no plan, and no verdict, so a reader must not select a backend or refuse a run from it. Its values map the bind modes of `fps-backend-afxdp` (TRN-11):

| Value | Number | TRN-11 mode |
|---|---|---|
| `XDP_MODE_UNSPECIFIED` | 0 | Not reported: a port that no AF_XDP queue uses, a backend other than AF_XDP, or an agent of 0.2.4 or earlier |
| `XDP_MODE_GENERIC` | 1 | `XdpMode::Generic`: the program in the generic (skb) hook. Every device supports it, and it is the slowest |
| `XDP_MODE_NATIVE` | 2 | `XdpMode::Native` with `BoundMode::Copy`: the program in the driver, and the kernel copies each frame |
| `XDP_MODE_ZERO_COPY` | 3 | `XdpMode::Native` with `BoundMode::ZeroCopy`: the program in the driver, and the driver writes and reads the UMEM directly |

Field numbers 9 and 10 of `Capabilities` and 6 of `PortInfo` were free: the messages end at `labels = 8` and `mtu = 5`, and the file declares no `reserved` range. Nothing is renumbered.

A port with queues in more than one mode (the kernel decides the bind mode of each queue when the agent asks for `Auto`) reports the slowest of its queues, in the order `GENERIC`, `NATIVE`, `ZERO_COPY`, because a rate that depends on one slow queue is a rate of the slow mode. This rule is for the agent (AGT-22) and is not checked by the validator.

## REST changes

- `Agent.kernel_release` is an optional string with the meaning of the protocol field. It is absent for an agent of 0.2.4 or earlier and when the agent sent an empty string.
- `Agent.ports[].xdp_driver_mode` is an optional string of `generic`, `native`, or `zero_copy`, with the meaning of the protocol enum and absent for `XDP_MODE_UNSPECIFIED`.
- `Agent.backend` keeps its enum (`kernel`, `af_xdp`, `dpdk`). Its text now says that it is the selected backend. A controller of 0.13.0 takes it from `selected_backend`, and for an agent that sends `BACKEND_UNSPECIFIED` it takes the first entry of `backends`, as before.

## Compatibility

The change is additive.

- `buf breaking` with the `FILE` rules reports no finding against `main`, because the change only adds fields with new numbers and a new enum (CI checks it, because `buf` is not installed in the sandbox of this package). The proto file compiles with `grpc_tools.protoc`. The REST change adds optional response properties and a description, and removes or changes no type or enum value, so the `openapi-breaking` workflow (oasdiff) finds nothing breaking (CI checks it, because `oasdiff` is not installed in the sandbox of this package).
- Wire level. An agent of 0.2.4 skips nothing it needs: the controller sends no new command or field to the agent. A controller of 0.2.4 skips the three fields of an agent of 0.2.5 and keeps showing `kernel`. REST clients that ignore the new properties see no difference, except that `Agent.backend` of an AF_XDP agent now reads `af_xdp`.
- The proto3 enum field `selected_backend` is a plain `Backend` value, so the generated Go field is `agentpb.Backend` and the Rust field is `i32`. A literal `agentpb.Capabilities{...}` or `agentpb.PortInfo{...}` that names no new field still compiles. The Rust mirrors in `engine/fps-agent/src/proto.rs` and `tests/contract.rs` list every field on purpose, so the contract test fails until AGT-22 adds the three fields there.

## Consumers

The order is the order of the work. Nothing before step 2 compiles against the new fields, and step 3 needs the code of step 2.

| Step | Consumer | What it does with the change |
|---|---|---|
| 1 | Agent, `engine/fps-agent` (AGT-22) | Fills the fields: `kernel_release` from `uname -r` (`/proc/sys/kernel/osrelease`), `selected_backend` from the selection of TRN-14, and `xdp_driver_mode` of each port from `fps_backend_afxdp::PortInfo::xdp_mode` and the bound mode of its queues, by the table above. Adds the three fields to the Rust mirrors in `src/proto.rs` and `convert.rs` and to the field tables of `tests/contract.rs`. Reports `ENGINE_API` 0.2.5 (and `tests/detect.rs`) once it sends the fields |
| 2 | Coordinator, regeneration | Runs the regeneration below after this contract merges, and pushes it to the branch of AGT-22 and CTL-53 |
| 2 | Controller, agent protocol client (`controller/internal/agents/agentpb`) | Regenerated from the proto file with the workflow `regen-agentpb` |
| 2 | Controller, REST (`controller/internal/api`) | `APIVersion` 0.13.0 in `capabilities.go`, and `api.gen.go` regenerated with `go generate ./internal/api`. `TestCapabilitiesVersions` and the `info.version` check in `capabilities_test.go` fail until then |
| 2 | UI (`ui/src/api/schema.d.ts`) | Regenerated with `npm run gen:api`. No page reads the new properties today, so no component changes. The agents view (UI-06) may show them later |
| 2 | SDK (`sdk/python/fixingpoint/_generated.py`) | Regenerated with `python3 sdk/python/scripts/generate_client.py`. `API_VERSION` becomes 0.13.0 and the TypedDict `Agent` gains `kernel_release` and the port key `xdp_driver_mode` |
| 3 | Controller, REST mirror (`controller/internal/agents/rest.go`, CTL-53) | Sets `Agent.backend` from `Capabilities.selected_backend` through the `backends` map, and falls back to the first entry of `backends` for `BACKEND_UNSPECIFIED`. Sets `Agent.kernel_release` when it is not empty, and `ports[].xdp_driver_mode` through a map from `XdpMode` to the REST enum when it is not `XDP_MODE_UNSPECIFIED`. Its test covers an agent of 0.2.4 and one of 0.2.5 |
| 3 | Fake agent (`controller/internal/fakeagent/fakeagent.go`) | May fill the fields in `DefaultCapabilities` to test the pass through |
| | CLI (`cli/`) | No change. The CLI has no code that reads `Agent.backend` or `ports` |

`plan/work-packages.yaml` has no package for step 3 today. The coordinator adds CTL-53 with the paths `controller/internal/agents/`, the dependency CON-32, and the order after AGT-22.

Regeneration for the coordinator, in one pull request after this one merges:

1. Run the workflow `regen-agentpb` (Go bindings of the proto file).
2. `go generate ./internal/api` in `controller/`, and set `APIVersion` to `0.13.0` in `controller/internal/api/capabilities.go`.
3. `npm run gen:api` in `ui/`.
4. `python3 sdk/python/scripts/generate_client.py`.

The Rust side needs no regeneration: `engine/fps-agent/build.rs` compiles the proto file with protox at build time, and the hand written mirrors are AGT-22's edit (step 1).

## Verification

`python3 tools/contracts/validate.py` passes, `python3 -m pytest tools/contracts` passes (354 passed, 1 skipped), the proto file compiles with `grpc_tools.protoc`, and `python3 tools/plan/plan.py check` passes. The pull request CI checks `buf`, `oasdiff`, and the workspace tests. The jobs that read `APIVersion` and `ENGINE_API` fail on this branch until step 1 and step 2 land, which is the order above.
