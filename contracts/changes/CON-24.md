# CON-24 Capture drop counts in the agent protocol and the REST API

Contract. Agent control protocol `proto/fps/agent/v1/agent.proto` and controller OpenAPI document `contracts/openapi/fps-api-v1.yaml`. The change follows ENG-27, which split the drop counters of `fps-capture`. The owner approved it on 2026-11-10 and asked for an additive change.

Problem. `StopCaptureResult.dropped` said "the number of packets that the capture missed", and the REST `CaptureFile.dropped` said "Packets that the agent dropped". Both count sessions, because the agent cannot know how many packets a lost session held. Each also mixed three causes that mean different things to an operator: sessions that ended while the writer queue was full (the capture was too slow), sessions that did not fit into one ring file, and sessions still open when the capture stopped (the stop cut them off, no loss of the writer). `fps-capture` counts them apart since ENG-27 as `sessions_dropped_overflow`, `sessions_dropped_oversize`, and `sessions_cut_at_stop`, and `engine/fps-agent/src/control.rs` still sends their sum as `dropped`.

Versions.

- The agent protocol moves from 0.2.3 to 0.2.4. The change adds three fields and no command, so it takes a patch number for the reason that CON-11, CON-13, and CON-15 gave: the controller admits agents by the major and minor part of `Capabilities.engine_api`, and a 0.3.0 would make it refuse every agent until its list changes. The header comment of the proto file, `buf.yaml`, and `contracts/README.md` name the new version. The Rust agent reports `ENGINE_API` 0.2.4 once it sends the fields, because `engine/fps-agent/tests/contract.rs` ties `ENGINE_API` to the header comment.
- OpenAPI moves from 0.10.0 to 0.11.0. The change adds optional response properties, which a reader can see in the document, so it takes a minor number, as CON-12, CON-14, and CON-15 did. `api.APIVersion` follows, because `capabilities_test.go` compares it with `info.version`. `openapi/fps-metrics.yaml` stays at 0.3.0.
- The metric catalog, the scenario contract, the test configuration, and the module manifest schema stay as they are.

Protocol changes. `StopCaptureResult` gains three fields. They are `optional`, so a reader tells an agent of 0.2.3 or earlier (field absent) from an agent that reports 0.

| Field | Number | Meaning |
|---|---|---|
| `dropped_overflow` | 6 | Sessions that ended while the writer queue was full. This is `CaptureStats.sessions_dropped_overflow` |
| `dropped_oversize` | 7 | Sessions that could not fit into one ring file. This is `CaptureStats.sessions_dropped_oversize` |
| `cut_at_stop` | 8 | Sessions still open when the capture stopped, or that ended after the writer had ended. This is `CaptureStats.sessions_cut_at_stop`. A capture that stops at rest reports 0 |

`dropped` (4) keeps its number and its type. Its text now says sessions, not packets, and says that it is the sum of the three fields. For an agent of 0.2.4 or later, `dropped == dropped_overflow + dropped_oversize + cut_at_stop` holds. An agent of 0.2.3 or earlier sends `dropped` alone, with the same meaning. Field numbers 6, 7, and 8 were unused, and the file declares no `reserved` range. Nothing is renumbered.

REST changes. `CaptureFile` gains the optional integer properties `dropped_overflow`, `dropped_oversize`, and `cut_at_stop` (minimum 0), with the same meaning and the same rule as the protocol fields. Each is absent until the agent reported the end, and absent for an agent of agent protocol 0.2.3 or earlier. The text of `dropped` now says sessions and the sum rule. `Capture` itself has no drop count (it has `bytes` only, and the per agent counts live in `files`), so it gets no new property.

Compatibility. The change is additive.

- `buf breaking` with the `FILE` rules reports no finding against `main`, because the change only adds fields with new numbers (CI checks it, because `buf` is not installed in the sandbox of this package). The proto file compiles with `grpc_tools.protoc`. The REST change adds optional response properties and changes descriptions, and removes or changes no type, so the `openapi-breaking` workflow (oasdiff) finds nothing breaking (CI checks it, because `oasdiff` is not installed in the sandbox of this package).
- Wire level. An agent of 0.2.3 skips nothing it needs: the controller sends no new command or field to the agent. A controller of 0.2.3 skips the three fields of an agent of 0.2.4 and keeps reading `dropped`, which still is the sum. REST clients that ignore the new properties see no difference, except that the documented unit of `dropped` is sessions.
- The proto3 `optional` keyword makes the generated Go fields pointers (`*uint64`) and the Rust fields `Option<u64>`. A literal `agentpb.StopCaptureResult{...}` that names no new field still compiles, and a Rust struct literal of `pb::StopCaptureResult` needs the three fields (`None` is enough).

Consumers.

| Consumer | What it does with the change |
|---|---|
| Agent, `engine/fps-agent/src/control.rs` and `session.rs` | Fills the three fields from `CaptureStats` (`sessions_dropped_overflow`, `sessions_dropped_oversize`, `sessions_cut_at_stop`) and keeps `dropped` as their sum. Reports `ENGINE_API` 0.2.4 |
| Controller, agent protocol client (`controller/internal/agents/agentpb`) | Regenerated from the proto file (workflow `regen-agentpb`) |
| Controller, capture code (`controller/internal/captures`) | Keeps the three values of `StopCaptureResult` in `AgentFile` and writes them to `CaptureFile` in `fileToAPI`, only when the agent sent them. `dropped` stays the value of the agent |
| Controller, REST (`controller/internal/api`) | `APIVersion` 0.11.0, and `api.gen.go` regenerated with `go generate ./internal/api` |
| Fake agent (`controller/internal/fakeagent/capture.go`) | May fill the fields to test the pass through |
| UI (`ui/src/api/schema.d.ts`) | Regenerated with `npm run gen:api`. No page reads `dropped` of a capture file today, so no component changes |
| SDK (`sdk/python/fixingpoint/_generated.py`) | Regenerated with `python3 sdk/python/scripts/generate_client.py`. The TypedDict `CaptureFile` gains the three keys |
| CLI (`cli/`) | No change. The CLI has no code that reads a capture file or `dropped` |

The exact files and edits for the coordinator are in the follow up of the log entry `docs/wiki/log/2026-11-10T2330-ingest-con-24-capture-drop-counts.md` and in the pull request.
