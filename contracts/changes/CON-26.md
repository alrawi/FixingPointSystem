# CON-26 A read-scoped capture download answers 409 for a running capture

Contract. Controller OpenAPI document `contracts/openapi/fps-api-v1.yaml`, operations `downloadCapture` and `stopCapture`. The owner approved the change on 2026-11-11.

Problem. `downloadCapture` needs scope read, and until CTL-44 it ended a capture that still ran before it served the files. `stopCapture` needs scope run for exactly that effect, so a read-only token could cut an operator's capture short (issue #452). CTL-44 (PR #468) changed the controller: without scope run, a download of a capture whose run has not ended and whose files are still written answers 409 and sends no command. The contract did not say so. The 409 fell under the default problem response, and the `stopCapture` text still said that a download ends a running capture.

Versions.

- OpenAPI moves from 0.11.0 to 0.12.0. The change documents a response status and the scope rule of an operation, which a reader can see in the document, so it takes a minor number, as CON-22 did for the documented 409 of `reserveAgent`. `api.APIVersion` follows, because `capabilities_test.go` compares it with `info.version`. `openapi/fps-metrics.yaml` stays at 0.3.0.
- The agent protocol, the metric catalog, the scenario contract, the test configuration, and the module manifest schema stay as they are.

REST changes.

- `downloadCapture` gains a description of the rule and a `409` response with a `Problem` body: the capture still runs, its run has not ended, and the caller lacks scope run. The capture keeps running.
- `stopCapture`: the sentence "Downloading a capture that still runs also ends it" now adds "when the caller has scope run".

Compatibility. No schema changes. The 409 was already possible through the default problem response since CTL-44, so a client that handles problems sees no new behaviour from the contract change itself. A read-scoped client that relied on a download ending a running capture was already broken by the security fix, and now finds the rule documented. The `openapi-breaking` workflow (oasdiff) reports no breaking change for an added response status.

Consumers.

| Consumer | What it does with the change |
|---|---|
| Controller, REST (`controller/internal/api`) | `APIVersion` 0.12.0, and `api.gen.go` regenerated. The handler in `controller/internal/captures/rest.go` already answers 409 (CTL-44) |
| UI (`ui/src/api/schema.d.ts`) | Regenerated with `npm run gen:api`. The capture download link could show the problem detail on 409 |
| Python SDK (`sdk/python`) | Regenerated with `scripts/generate_client.py`. A 409 raises the SDK's problem error as before |
