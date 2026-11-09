# CON-12 Side selection in the run metrics API

Contract. Controller OpenAPI document `contracts/openapi/fps-api-v1.yaml`. The change completes CTL-23, whose metric views default to the client side while the aggregator can serve the server side.

Versions. OpenAPI moves from 0.3.0 to 0.4.0. Earlier changes that only added optional fields and named schemas, such as CON-07, took a minor number, and a patch number went only to a change that adds nothing a reader can see in the document, such as the annotations of 0.1.1. This change adds an optional query parameter, so it takes a minor number. Nothing depends on the minor part of the REST version the way the controller depends on the minor part of `engine_api` (see CON-11), so the bump locks nobody out. `api.APIVersion` follows, because `capabilities_test.go` compares it with `info.version`. `openapi/fps-metrics.yaml` stays at 0.3.0, because `GET /metrics` is unchanged.

Compatibility. Omitting the parameter keeps today's answer: the series count the client side, exactly as before. A client that never sends `side` sees no difference, and every request and response that was valid stays valid. The change is additive.

- `getRunMetrics` (`GET /v1/runs/{id}/metrics`) gains the optional query parameter `side` with the values `client` and `server` and the default `client`.
- `streamRunMetrics` (`GET /v1/runs/{id}/metrics:stream`) takes the same parameter, because it serves the same views from the same aggregator. One `side` applies to every requested view of the stream, and omitting it keeps the client side. The two operations share the vocabulary, and a stream that could not select the server side would leave the live and the stored view disagreeing.
- Any other value answers 400. The response schemas are unchanged, and a response does not echo the side.
- The server side exists for the metrics that carry the `side` label of CON-08, which are the scenario level and transport metrics. A total over both sides counts each session twice, so no value selects both.
- Generated code. `ui/src/api/schema.d.ts` and `sdk/python/fixingpoint/_generated.py` are regenerated, and `get_run_metrics` in the SDK takes `side`. `controller/internal/api/api.gen.go` must be regenerated with `go generate ./internal/api`, which the coordinator does on the branch.
- Outside the catalog paths, `controller/internal/metrics/rest.go` passes `side` to `Window.Side` and `Aggregator.LatestSide`, and `controller/internal/api/capabilities.go` names version 0.4.0. Both are the minimum that makes the parameter work.
