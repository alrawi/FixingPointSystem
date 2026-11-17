## [2026-11-17] ingest | CON-32 Kernel release, selected backend, and XDP mode in the agent capabilities

CON-32 (issue #576, issue #74 in part) adds `Capabilities.kernel_release` (9), `Capabilities.selected_backend` (10), `PortInfo.xdp_driver_mode` (6), and the enum `XdpMode` to the agent protocol, which moves from 0.2.4 to 0.2.5, and mirrors them in the REST `Agent` as `kernel_release` and `ports[].xdp_driver_mode`, which moves OpenAPI from 0.12.0 to 0.13.0. It is an additive change that the vetting agent approved under D13 on 2026-11-16 (vetted by Fable 5.2). `contracts/changes/CON-32.md` holds the reasons, the compatibility, and a Consumers table with AGT-22 first, then the regeneration by the coordinator, then CTL-53. [Contracts](Contracts.md) has a section for it.

Decisions.

- `XdpMode` maps the bind modes of TRN-11: the generic hook is `GENERIC`, the driver hook with copy mode sockets is `NATIVE`, and the driver hook with zero copy sockets is `ZERO_COPY`. The enum documents the value as a driver table hint: no command, plan, or verdict depends on it.
- The REST `ports[].xdp_driver_mode` is a lowercase enum (`generic`, `native`, `zero_copy`) and is absent when the agent sent `XDP_MODE_UNSPECIFIED`. `Agent.backend` stays an enum of `kernel`, `af_xdp`, `dpdk`.
- The NUMA fields of issue #74 stay open.

Findings.

- The edits outside `contracts/` that this change needs are in the Consumers table of the change entry: the version constants `APIVersion` in `controller/internal/api/capabilities.go` and `ENGINE_API` in `engine/fps-agent/src/capabilities.rs` (with `engine/fps-agent/tests/detect.rs`), and the regenerated bindings. `capabilities_test.go` and `engine/fps-agent/tests/contract.rs` fail on a stale constant until then.
- `plan/work-packages.yaml` names no package CTL-53 yet. The coordinator adds it, with the REST mirror in `controller/internal/agents/rest.go`.

### Shared page notes

None.
