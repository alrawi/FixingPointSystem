## [2026-11-12] ingest | CON-28 Packet gap series in the metric catalog

The metric catalog moves from 0.7.0 to 0.8.0 (`contracts/changes/CON-28.md`, approved by the owner on 2026-11-12). The label `property` of `realism.ks_statistic` and `realism.ks_p_value` gains `packet_gap` (both directions pooled), `request_gap` (client streams), and `response_gap` (server streams), so the packet gaps of RLM-15 reach `RunReport.realism` as series. The same pull request carries the consumers, which the coordinator authorized: the controller catalog copy, the version pins, the label values in `engine/fps-stats/src/catalog.rs`, the realism runner (`realism/temporal.py` emits the series, report format 0.17.0, the report schema, the controller copy of the schema, and `ui/src/api/schema.d.ts`), the mapping of the new values in `controller/internal/report/realism.go`, the example counts of `realism_con23_test.go` (23 cases), and the pages [Contracts](Contracts.md) and [Realism](Realism.md) with `realism/README.md`.

Decisions.

- `payload_size` is not a catalog value. The sizes already have `request_bytes`, `response_bytes`, and `both` (CON-22 names the first two as the `payload_size` comparisons by direction), so a second name would duplicate the series. The runner still does not emit `both` for the sizes of both directions, which needs no contract change and stays a later RLM change.
- A paced comparison (the `constant` target of a packet gap) carries no KS series: its statistic is the share of gaps outside the tolerance and its p value is 0 or 1, which are not what the metrics describe. `realism.check_failures` shows its failure.
- The direction is part of the value (`request_gap`, `response_gap`, `packet_gap`), as for the sizes, so the catalog needs no new label and two comparisons of one application never share a series.

### Shared page notes

- [Module Development](Module-Development.md): none. `temporal.packets` of the labels file is unchanged, so a module author writes no fixture label differently.
- [Testing](Testing.md): none. The tests are in `realism/tests/test_packet_gaps.py` and `realism/tests/test_check_metrics.py`.
- Report format: this change took 0.17.0 after RLM-15 took 0.16.0. RLM-16 may also bump the format, so the coordinator checks the number at merge.
