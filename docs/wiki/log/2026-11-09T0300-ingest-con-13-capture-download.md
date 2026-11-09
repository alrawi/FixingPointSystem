## [2026-11-09] ingest | CON-13 capture download over the agent connection

The owner chose to stream a finished capture over the existing agent connection, and CON-13 defines it in protocol 0.2.2. The agent dials the controller, so the controller asks with the new command `fetch_capture` on `Session`, and the agent answers with the size and digest and then opens the new client streaming call `UploadCapture` on the same connection. We took this shape and not chunks on `Session` because the upload is a separate HTTP/2 stream with its own flow control, so it needs no credit scheme and cannot hold up heartbeats. Chunks carry an offset and at most 256 KiB, the end message carries the total size and the digest that `StopCaptureResult` reported, and the controller resumes by sending a new command with the offset it holds. We chose a patch number because the controller admits agents by major and minor. The Rust agent reports 0.2.2 and answers `ERROR_CODE_UNIMPLEMENTED` until AGT-09. We documented the change in [Contracts](../Contracts.md#capture-download-over-the-agent-connection).

### Shared page notes

- [Agent](../Agent.md): the section "How the controller gets the file" and the known gaps still say that CON-05 defines no transfer. CON-13 defines it, AGT-09 serves it, and the agent reports `engine_api` 0.2.2.
- [Controller](../Controller.md): CTL-14 sends `fetch_capture` only to agents that report 0.2.2 or later, stores the uploaded bytes, and verifies the digest of the whole file.
- Testing: the Go test job fails on the generated code currency test until the coordinator regenerates `controller/internal/agents/agentpb` on this branch.
