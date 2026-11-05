# CON-07 API contract fields for failure reasons and cleanup

Contract. Controller OpenAPI document `openapi/fps-api-v1.yaml` and the new document `openapi/fps-metrics.yaml`.

Versions. OpenAPI moves from 0.2.0 to 0.3.0, because it adds response fields and named schemas. Every change is additive, and every response that was valid stays valid. The change applies decision D12 items 19 and 20.

- `Run.reason` says in words why a run failed or was aborted, and is absent in every other state.
- The run verdict becomes the named schema `RunVerdict`, and its criteria become the named schema `CriterionResult`. The wire format of `observed` and `passed` is unchanged, and the description now states when each is absent. The new field `reason` says why the criterion passed, failed, or has no result. Named schemas give generated clients one type in place of an inline structure that each caller repeats.
- Every enum names `type: string`, so the generator workaround in `controller/internal/api/contract` no longer changes the document. The rewrite stays in the controller as a guard.
- `MetricSample.values` carries format double, so counters above 2^24 keep their exact values in generated clients.
- `openapi/fps-metrics.yaml` documents `GET /metrics` on the controller root, outside the `/v1` base path. It is open by default and accepts the optional bearer scheme `metricsToken`, which a controller setting enables (D12 item 2, CTL-19). The endpoint lives in its own document because every operation of `fps-api-v1.yaml` sits under `/v1` and needs a token scope, and the controller generator and the validator rely on both rules. The description of `fps-api-v1.yaml` points at it.
- The controller code in `controller/internal/api` and the UI client in `ui/src/api/schema.d.ts` are regenerated. `runs.ToAPI` builds the named types and `api.APIVersion` is 0.3.0. No handler fills the new fields yet, which CTL-20 does.
- The `contracts/README.md` file table may add a row for `openapi/fps-metrics.yaml` when the coordinator folds this entry.
