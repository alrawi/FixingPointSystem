## [2026-11-10] ingest | CON-20 Stream events, metric source label, and provenance fields

We documented the events of `GET /v1/runs/{id}/metrics:stream` in the REST contract (the first `retry` field, the tick ids, the `end` event with its `MetricStreamEnd` data, no replay, and the 501 of a controller without a stream), added the label `source` to the three mix metrics of the metric catalog, and added the optional provenance fields `capture_sha256`, `scrub_rules`, and `scrubbed_at` to the module manifest schema. The owner approved the change on 2026-11-10, and every part is additive. OpenAPI moves to 0.8.0, the catalog to 0.4.0, and the manifest schema to v0.4. We updated [Contracts](../Contracts.md) and the entry `contracts/changes/CON-20.md`.

### Shared page notes

- [Controller](../Controller.md): the stream section can cite the contract text now. The controller sends the first `retry` field, `id` equal to the tick number, and the `end` event, as the operation describes, and it ignores `Last-Event-ID`.
- Coordinator, outside the catalog paths of CON-20: refresh `controller/internal/metrics/catalog.yaml` (byte copy), set `APIVersion` to 0.8.0 in `controller/internal/api/capabilities.go`, regenerate `api.gen.go`, update the mirror `engine/fps-stats/src/catalog.rs` (labels of `mix.*`, `LABEL_NAMES`, `LABEL_VALUES`), change `test_catalog_is_0_3_0_with_property_both` in `tools/contracts/test_contracts.py` to 0.4.0, and regenerate `sdk/python/fixingpoint/_generated.py` and `ui/src/api/schema.d.ts`.
- Follow up: RLM can report `mix.share_pct` and `mix.abs_error_pp` with `source=classifier` in the `metrics` list of `mix_accuracy`, and CAP can write the three provenance fields instead of the sentence in `notes`.
