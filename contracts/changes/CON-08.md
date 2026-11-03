# CON-08 Metric catalog views, reasons, and side label

Contract. Metric catalog in `contracts/metrics/catalog.yaml` and its schema `contracts/metrics/catalog.schema.json`.

Versions. The catalog moves from 0.1.0 to 0.2.0, because it gains fields and label values. Every metric keeps its name, type, unit, and level, so readers of 0.1.0 stay valid. The OpenAPI document is unchanged.

- Every metric names its API view in the new required field `view`, one value of the OpenAPI `MetricView` enum, as decision D12 item 13 asks. Application level metrics map to `application`, the realism gauges to `realism`, scenario and flow action metrics to `scenario`, `tcp.*` to `tcp`, `tls.*` to `tls`, and port and host metrics to `port`. The application view also serves the scenario metrics summed over the `scenario` label. The schema checks that each view fits the level of its metric.
- The new required top level field `histogram_statistics` holds the fixed list `p50`, `p90`, `p99`, `p999`, `min`, `max`, `mean`, and `count` of D12 item 15. The schema pins the list with `const`, and it equals `HISTOGRAM_STATS` in `tools/contracts/validate.py`.
- The `reason` label gains `datagram_rejected`, `template_error`, and `extension_error` after `internal`, as D12 item 17 asks.
- Every scenario level metric and every transport metric gains the `side` label with the values `client` and `server`, as D12 item 21 asks. The label follows `scenario` or `port` in each label list. A total over both sides counts each session twice, so the API views and test criteria report the client side unless a criterion selects the server side. `bytes.tx` and `bytes.rx` now describe the bytes that the side sent and received, which equals their old meaning on the client side.
- The engine mirrors in `engine/fps-stats/src/catalog.rs` and `Reason` in `engine/fps-interp/src/failure.rs` follow the new labels and reasons, because their contract tests compare them with the catalog. No engine code reports the new reasons or the side label yet, which ENG-14 adds.
