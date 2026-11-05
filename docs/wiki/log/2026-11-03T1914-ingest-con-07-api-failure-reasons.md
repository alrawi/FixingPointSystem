## [2026-11-03] ingest | CON-07 API contract fields for failure reasons and cleanup

We applied decision D12 items 19 and 20 to the controller API and documented the result in the section Failure reasons and the metrics endpoint of [Contracts](Contracts.md). OpenAPI moves to 0.3.0 with additive changes only. `Run.reason` says why a run failed or was aborted. The verdict and its criteria became the named schemas `RunVerdict` and `CriterionResult`, and each criterion result gained `reason` beside `observed` and `passed`. Every enum names `type: string`, and `MetricSample.values` carries format double. The new document `contracts/openapi/fps-metrics.yaml` describes `GET /metrics` on the controller root, open by default and with an optional bearer scheme for the token setting of CTL-19. We regenerated `controller/internal/api` and `ui/src/api/schema.d.ts`, changed `runs.ToAPI` to build the named types, and set `api.APIVersion` to 0.3.0. No handler fills the new fields, which CTL-20 does. The entry `contracts/changes/CON-07.md` records the change, and the evidence is `docs/handoff/CON-07.md` on the branch `wp/CON-07`.

## Shared page notes

The coordinator may apply the notes below.

1. `contracts/README.md` may add a row for `openapi/fps-metrics.yaml` to its file table, with the text "Prometheus exposition at `GET /metrics` outside `/v1` (OpenAPI 3.1)".
2. [Implementation Plan](Implementation-Plan.md) may mark CON-07 as done after the merge, which unblocks CTL-20 together with CON-09.
