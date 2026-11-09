## [2026-11-09] ingest | CON-15 capture and criteria contract gaps

The OpenAPI document moves to 0.6.0 and the agent protocol to 0.2.3, both additively. REST gains `stopCapture` (`POST /runs/{id}/captures/{capture_id}:stop`) and the optional `Capture.files` with the agent of each file. The protocol gains `StartCaptureCommand.export_tls_keys` and `Criterion.labels`. `contracts/examples/tests/dns_zone.yaml` uses `dns/lookup_a`, and `tools/contracts/validate.py` validates it against the first party `modules/dns`. The capture `port` stays required, because AGT-12 makes the agent filter by it. See [Contracts](../Contracts.md#capture-and-criteria-gaps) and `contracts/changes/CON-15.md`.

### Shared page notes

- Controller page: the lines that say the REST contract has no stop operation and that `export_tls_keys` has no effect are stale once CTL serves them. The contract has both from 0.6.0 and 0.2.3.
- Agent page: the agent reports 0.2.3, ignores `export_tls_keys` (it writes the key lines whenever it has them), and refuses a plan with a labeled criterion until an AGT package applies `Criterion.labels`. The note about "no labels on the wire Criterion" in the AGT-07 entry is resolved on the contract side.
- Testing: `engine/fps-agent/tests/contract.rs` checks the field numbers and the round trips of the two new fields.
- Follow-up work, for the coordinator to plan: CTL serves `stopCapture`, fills `Capture.files` from `captures.AgentFile`, sends `export_tls_keys`, and sends labeled criteria only to agents of 0.2.3 or later; AGT applies `Criterion.labels` and `export_tls_keys`.
- Go code the coordinator regenerates (`agentpb`, `api.gen.go`) then needs: `StopCapture` in `controller/internal/api/unimplemented.go`, `APIVersion` 0.6.0 in `capabilities.go`, `stopCapture` in `pendingOps` of `wiring_test.go`, and `dns/lookup_a` handling in `wiring/settings_test.go` and `store/locations_test.go` (they load `dns_zone.yaml` against the example modules, which have `resolve` and no `lookup_a`).
- Implementation Plan and the work package list: CON-16 depends on CON-15, which is done once merged.
