## [2026-11-09] ingest | CON-12 side selection in the run metrics API

`GET /v1/runs/{id}/metrics` and `GET /v1/runs/{id}/metrics:stream` take the optional query parameter `side` (`client` or `server`, default `client`), and the controller API moves from 0.3.0 to 0.4.0. We chose a minor number because the change adds a parameter, as CON-07 added fields, and the REST version has no admission rule like the `engine_api` rule of CON-11. Omitting `side` keeps the client side answer. The handler in `controller/internal/metrics/rest.go` passes the side to `Window.Side` and `Aggregator.LatestSide`, and `api.APIVersion` names 0.4.0. The UI schema and the Python SDK are regenerated, and `api.gen.go` waits for the coordinator's regeneration. We documented the change in [Contracts](../Contracts.md#side-selection-in-the-run-metrics-api).

### Shared page notes

- [Controller](../Controller.md): the metrics endpoints accept `side`, and the controller API is at 0.4.0.
- Testing: the Go test job fails on the generated code currency test until `api.gen.go` is regenerated on this branch.
