## [2026-11-03] ingest | CON-08 Metric catalog views, reasons, and side label

We moved the metric catalog to version 0.2.0. Every metric names its API view, the histogram statistic suffixes form a fixed list in the catalog, the `reason` label gains `datagram_rejected`, `template_error`, and `extension_error`, and session and transport metrics gain the `side` label. The engine mirrors of the labels and reasons follow, because their contract tests read the catalog. [Contracts](Contracts.md) has a new section, and evidence is in `docs/handoff/CON-08.md` and `contracts/changes/CON-08.md`.

### Shared page notes

- `controller/internal/metrics/catalog.yaml` must become a byte copy of `contracts/metrics/catalog.yaml` again. The controller workflow does not run on catalog changes, so the copy test `TestCatalogMatchesContract` in `controller/internal/metrics/prometheus_test.go` fails on the next controller pull request until a CTL package copies the file. The controller tests pass with the copied file.
- `tools/contracts/validate.py` keeps its own `HISTOGRAM_STATS` list. A small follow up could read the list from `histogram_statistics` in the catalog, so the two can never drift.
- The controller workflow could add `contracts/metrics/**` to its path filters, so a catalog change runs the copy test.
