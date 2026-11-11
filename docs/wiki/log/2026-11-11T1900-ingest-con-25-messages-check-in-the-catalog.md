## [2026-11-11] ingest | CON-25 The messages check in the metric catalog

RLM-14 added the `messages` check to the realism report after CON-23 listed the eight check kinds, so the label `check` of the catalog had no value `messages` and a failing messages check never reached `RunReport.realism`. The owner approved the contract change on 2026-11-11 (comment on #458). The catalog moves from 0.5.0 to 0.6.0 and `check` gains the value `messages`, which is the whole change. `realism.check_failures` for the check is its `failures_total`, the check carries no other series, and the generic CON-23 mapping (`== 0`, the `passed` of the check) covers it. `contracts/examples/realism/check-metrics.yaml` gains a passing case (0) and a failing case (2). The entry `contracts/changes/CON-25.md` lists the consumers: the realism runner, the `fps-stats` tables, and the controller catalog copy. `validate.py` passes. `tools/contracts/test_contracts.py` fails on two tests until the coordinator applies the follow up (the version pin and the byte copy), as it did for CON-23.

### Follow up outside contracts/ (coordinator)

1. `cp contracts/metrics/catalog.yaml controller/internal/metrics/catalog.yaml`.
2. `tools/contracts/test_contracts.py`, `test_catalog_is_0_3_0_with_property_both`: `"0.5.0"` becomes `"0.6.0"`.
3. `realism/tests/test_mix.py`, `test_the_series_validate_against_catalog_0_4_0`: `"0.5.0"` becomes `"0.6.0"`.
4. `engine/fps-stats/src/catalog.rs`: append `"messages"` to the values of `check` in `LABEL_VALUES`. No metric id changes.
5. RLM package: `realism/messages.py` emits `realism.check_failures` with `check: messages` and `failures_total`, the schema and the failures rule include the check, the format moves to 0.14.0 (RLM-10 took 0.13.0 first), and `realism/tests/test_check_metrics.py` loads the new example.

### Shared page notes

- `docs/wiki/Realism.md` (lines about the `messages` check carrying no series) is for the RLM package that emits the series.
