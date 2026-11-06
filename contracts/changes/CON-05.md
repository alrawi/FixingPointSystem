# CON-05 Agent control protocol v0.2

Contract. Agent control protocol in `proto/fps/agent/v1/agent.proto`.

Versions. The protocol moves from 0.1.0 to 0.2.0. The header comment of the proto file and the comment in `buf.yaml` name the new version. `Capabilities.engine_api` is how an agent announces the version it implements, so an agent that reports 0.1.0 never receives the new commands.

Changes.

- `SessionResponse.body` gains `update_rates` (6, `UpdateRatesCommand`), `start_capture` (7, `StartCaptureCommand`), and `stop_capture` (8, `StopCaptureCommand`).
- `CommandResult.result` gains `update_rates` (6, `UpdateRatesResult`), `start_capture` (7, `StartCaptureResult`), and `stop_capture` (8, `StopCaptureResult`).
- `UpdateRatesCommand` carries the plan id and a list of `RateUpdate` values. Each value names a mix entry and gives the new steady state target of the whole agent, as `sessions_per_second` or `concurrent_sessions`. The agent divides the target among its workers in proportion to their current `WorkerRate` values, and a mix entry that the command does not name keeps its target. The command is valid in `AGENT_STATE_RUNNING` only, and it is applied completely or not at all.
- `StartCaptureCommand` carries the plan id, a controller chosen `capture_id`, an optional port name, a pcap-filter expression, a snap length, and byte and time limits. `StopCaptureCommand` names the capture and is idempotent. `StopCaptureResult` reports packets, bytes, dropped packets, and a `sha256:` digest of the pcapng file.
- `ModuleRef` gains `signature` (4, bytes) and `signature_key_id` (5, string). The signature is an Ed25519 signature of 64 bytes over the UTF-8 bytes of the `digest` field, and both fields stay empty for an unsigned module. They sit outside the module tree, so no digest changes.
- Every failure uses the existing `ErrorCode` values, so `ErrorCode` and `contracts/README.md` stay as they are.

Field numbers kept free. `ModuleRef` field number 6 stays unused for `settings`, which CON-11 adds. A comment in the proto file names the number. The file declares no `reserved` range, so CON-11 adds the field without removing anything.

Compatibility. The change is additive on the wire and in the proto file.

- `buf breaking` with the `FILE` rules reports no finding against `origin/main` at 5b612d2, which holds v0.1. A control run that changed `ModuleRef.digest` from 3 to 9 reported the deleted field, so the check does detect breaks. `buf format` and `buf lint` pass.
- Every v0.1 message, field number, enum value, and RPC keeps its meaning. `Register` and `Session` are unchanged, and the service has no new RPC.
- Wire level: a v0.1 agent that reads a v0.2 `ModuleRef` ignores the two new fields, and a v0.2 agent that reads a v0.1 `ModuleRef` sees empty signature fields. A v0.1 agent cannot decode the three new command arms, which arrive as unknown fields with no `body` set, so the controller must send them only to an agent whose `engine_api` is 0.2.0 or later.
- JSON mapping: `contracts/examples/plans/http_dns_smoke.json` stays valid and canonical, because the new fields have default values that the canonical form omits.
- Source level: the change is not source compatible for code that builds `ModuleRef` with every field listed and for code that mirrors the proto by hand. The affected files are listed in the pull request, and the packages that implement the new fields own them.
- Not specified here: how the controller retrieves a capture file after `StopCaptureResult`, and how the controller learns the registry key for `signature_key_id`. The first needs an owner decision on the transfer path, and the second belongs to CTL-11 and AGT-04.
