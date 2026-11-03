## [2026-11-02] ingest | CON-02 Counter catalog v0.1

We added the metric catalog `contracts/metrics/catalog.yaml` with 48 metrics, its JSON Schema, and the validator rule that every metric in test criteria and in OpenAPI `x-fps-metric` annotations exists in the catalog. We added a Counter catalog section to [Implementation Plan](Implementation-Plan.md). Evidence is in `docs/handoff/CON-02.md` and `tools/contracts/test_contracts.py`.
