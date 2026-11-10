# CON-20 Stream events, metric source label, and provenance fields

Contract. Controller OpenAPI document `contracts/openapi/fps-api-v1.yaml`, metric catalog `contracts/metrics/catalog.yaml`, and module manifest schema `contracts/schemas/module-manifest.schema.json`. The change takes findings of UI-04, RLM-07, and CAP-03. The owner approved it on 2026-11-10. Every change is additive.

Versions.

- OpenAPI moves from 0.7.0 to 0.8.0. The change documents the events of a stream and a 501 response, which a reader can see in the document, so it takes a minor number, as CON-12 and CON-14 did. `api.APIVersion` follows, because `capabilities_test.go` compares it with `info.version`. `openapi/fps-metrics.yaml` stays at 0.3.0.
- The metric catalog moves from 0.3.0 to 0.4.0. It adds a label to three metrics, which a reader can see.
- The module manifest schema moves from v0.3 to v0.4. It adds three optional fields.
- The scenario contract (v0.6), the test configuration (v0.3), the agent protocol (0.2.3), and the extension world stay as they are.

Stream events (UI-04 finding). `streamRunMetrics`, `GET /v1/runs/{id}/metrics:stream`, now documents what the controller sends (`controller/internal/metrics/rest.go`) and what the UI reads (`ui/src/api/eventStream.ts`, `ui/src/runs/useRunMetrics.ts`). The text lives in the operation description, and the vendor extension `x-event-stream` lists the events for tools.

- `retry: <ms>` is the first field, the event period (1000 ms by default).
- Each tick sends, for each requested view in order, one event per current sample. The event has no `event` field (type `message`), the `id` is the tick number counted from 1 on each connection (all events of a tick share it), and `data` is one line with a `MetricSample`.
- The event named `end` follows the samples of the final tick when the run is `completed`, `failed`, or `aborted`, or the controller no longer knows the run. It has no `id`, and `data` is the new schema `MetricStreamEnd`, `{state}`, where `state` is a `RunState` value or `unknown`. The stream then closes, and a client must not reconnect.
- The controller ignores `Last-Event-ID` and does not replay ticks. A reconnect starts at tick 1, and a client reads `getRunMetrics` for the gap. A client that was disconnected before the end reads `getRun` for a final state, because no `end` event follows then.
- The headers `Cache-Control: no-store` (and `X-Accel-Buffering: no`) are part of the 200 answer.
- New response `501` with `application/problem+json`: a controller without a stream answers it, and a client polls `getRunMetrics` instead and does not retry. The wiring answers every operation that no package serves with this problem, so the status was already real. `default` still covers 400 (a bad `view` or `side`) and 404 (an unknown run).

Metric source label (RLM-07 finding). New label `source` with the values `metrics` (the counters of the run, as the controller reports them) and `classifier` (the sessions of the capture, as the classifier that the `classifier` label names labels them). `mix.share_pct` and `mix.abs_error_pp` use `[application, source, classifier]`, and `mix.max_abs_error_pp` uses `[source, classifier]`. The labels `source` and `classifier` are only set where the realism check reports a classifier measurement. A series without `source` is a run metrics series, as every series was before, so the controller, the engine, and every criterion with `where: {application: ...}` keep their meaning. The realism check (RLM) can now report its `mix_accuracy` metrics.

Provenance fields (CAP-03 finding). The `provenance` object of the module manifest gains three optional fields, which the scrub record of `capture scrub` already holds:

- `capture_sha256`: the SHA-256 of the capture file, 64 lowercase hexadecimal digits.
- `scrub_rules`: an array of `{id, version}` (identifier, integer of at least 1), the scrub rules that ran and the version of each at that time. Items are unique.
- `scrubbed_at`: the date (`YYYY-MM-DD`) or the UTC date and time (RFC 3339) of the scrub.

The three fields are optional for every source, so a manifest that has none stays valid, and the rule for `source: capture` (consent and `scrubbed: true`) is unchanged. The fields never carry a removed value.

Compatibility. A client that ignores the stream text, the 501, `source`, and the new fields sees no difference. `oasdiff breaking` reports nothing for the OpenAPI document, because the change adds a response, a schema, a vendor extension, and a header declaration. Existing manifests, test configurations, and criteria stay valid.

Follow up outside the catalog paths (the coordinator regenerates or applies them, and this package does not edit them).

- Controller: `controller/internal/metrics/catalog.yaml` is a byte copy of the catalog, and `tools/contracts/test_contracts.py::test_controller_copies_are_byte_identical` fails until it is refreshed. `controller/internal/api/capabilities.go` `APIVersion` moves to `0.8.0`. `api.gen.go` is regenerated with `go generate ./internal/api` (new schema `MetricStreamEnd`, no change to handlers).
- Engine: `engine/fps-stats/src/catalog.rs` mirrors the catalog, and `engine/fps-stats/tests/catalog.rs` compares it. The labels of the three mix metrics change, `LABEL_NAMES` gains `source`, and `LABEL_VALUES` gains `("source", &["metrics", "classifier"])`.
- Python SDK `sdk/python/fixingpoint/_generated.py` and `ui/src/api/schema.d.ts` follow the document (`MetricStreamEnd`, the 501).
- RLM: `realism/mix.py` can fill `metrics` with `mix.share_pct` and `mix.abs_error_pp` with `source=classifier`. CAP: `capture/scrub.py` and the scenario generator can write the three provenance fields instead of the sentence in `notes`. Both are separate packages.
