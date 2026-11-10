## [2026-11-10] ingest | CTL-40 The first response frame of an agent session

`Manager.Session` in `controller/internal/agents/service.go` now sends the response headers as soon as `m.open` returns, through the new method `session.sendHeader` (`stream.SendHeader(metadata.MD{})`). Before, Go's gRPC server sent them with the first message, so an idle session sent nothing, and the agent, which waits for the headers before it heartbeats, never heartbeated. The controller's sweep ended every idle session 5 to 6 s after it began, which is the cause that IMG-07 found in the journals of two failed boot tests. A command can send the first message before `sendHeader` runs. `session` therefore records that the headers went out (`headerSent`, under the send lock), and `sendHeader` does nothing then, because a second header write is an error. A failed header write detaches the session with the reason `stream closed`.

The regression test `TestIdleSessionSendsHeadersAtOnce` in `controller/internal/agents/headers_test.go` has a client that waits for the response headers before it heartbeats, as tonic does. It holds an idle session open for three heartbeat timeouts. The test commit went to the pull request alone first, and the job `test` of the workflow `ctl` failed on the old code with `the controller sent no response headers within 2 s of the open message ... Sweep after one heartbeat timeout ended [agent-h]` (run 42916193523, job 130179128966). The fix commit follows it.

After this change the session gate of `image/tests/vm-run.sh` (IMG-07) finds a `stable` session on the first try: the agent opens every session at once, heartbeats from the first second, and holds it for the 3 s that the gate asks for. The gate stays as a guard against a controller that regresses.

### Shared page notes

- [Controller](../Controller.md): the section "First response frame of a session" under "Agent manager" is in this pull request, and the CTL-40 work package owns it. Nothing else changes.
- [Agent](../Agent.md): the agent can still heartbeat while it waits for the response headers, as the IMG-07 note says. This is the agent's own change and not part of CTL-40.
- [Image](../Image.md): after the merge, the text of "The agent session gate" can say that the controller sends the headers at once (CTL-40) and that the gate now only guards against a regression. IMG-07 owns that page.
