# Contracts changelog

## 0.1.1 (2026-11-02)

- Added the metric catalog `metrics/catalog.yaml` (catalog 0.1.0) and its schema `metrics/catalog.schema.json`. The catalog names 48 metrics across the application, scenario, flow action, transport, and port and host levels (CON-02).
- OpenAPI 0.1.1 names the catalog metric of every `RunSummary` property through the `x-fps-metric` keyword and points criteria and metric sample values at the catalog. No field was added, removed, or renamed.
- The validator rejects metric names in test criteria and in `x-fps-metric` annotations that the catalog does not define, and checks the catalog itself.

## 0.1.0 (2026-11-02)

- First draft of the controller OpenAPI document, the module manifest schema, the scenario schema, and the test configuration schema.
- Example modules `http1` and `dns`, and example tests `enterprise_mix_10g` and `http_smoke`.
