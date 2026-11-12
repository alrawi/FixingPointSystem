## [2026-11-12] ingest | CTL-48 The CON-23 property wording for every realism metric

`report.MapRealism` words `property` of every mapped metric as the examples of `contracts/examples/realism/check-metrics.yaml` do (`failures of the temporal check`, `parser errors of http1 by tshark`, `decrypted TLS sessions`, `ttl deviations of client SYN packets`, `mix error of http1 by ndpi`, and the rest), in one function, `metricProperty`, which depends on the metric and its labels and not on the row that gives the target. CTL-46 had done it for the KS metrics and the conformance ratio only. A metric that the examples do not name, and a series without a label that its wording needs, keep the name followed by the labels. `TestMapRealismCON23Examples` no longer leaves `property` out of the comparison, so every case of `check-metrics.yaml` compares it. The wording is in [Controller](Controller.md#realism-reports-attached-to-a-run).

Decision: the wording no longer depends on whether the entry that gives the outcome was found. A conformance ratio or KS metric that finds no entry, which a conforming document never causes, now reads `conformance of other by stub` and not `realism.conformance_ratio of application other, classifier stub`, and `TestMapRealismFallback` follows.

### Shared page notes

- [Testing](Testing.md), [Home](Home.md), [Implementation Plan](Implementation-Plan.md), [Module Development](Module-Development.md): none.
- Counts: `realism_con23_test.go` pins 10 check kinds and 21 cases. CON-28 may add cases to `check-metrics.yaml`, so the merge with `origin/main` must update those numbers.
