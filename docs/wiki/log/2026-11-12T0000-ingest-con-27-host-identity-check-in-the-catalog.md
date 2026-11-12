## [2026-11-12] ingest | CON-27 The host identity check in the metric catalog

RLM-10 added the `host_identity` check after CON-25, so the label `check` of the catalog had no value `host_identity` and a failing check never reached `RunReport.realism`. The owner approved the contract change on 2026-11-11 (comment on #480). The catalog moves from 0.6.0 to 0.7.0 and `check` gains the value `host_identity`. `realism.check_failures` for the check is its `failures_total`, the check carries no other series, and the generic CON-23 mapping covers it. `contracts/examples/realism/check-metrics.yaml` gains a passing case (0) and a failing case (2). The entry `contracts/changes/CON-27.md` also settles the open question of CTL-42: a report holds at most one check of each kind, `passed` of the series is the `passed` of that check, and a document with two checks of one kind is invalid (the controller gives the finding for a repeated metric and labels pair and does not combine outcomes). `validate.py` passes.

The coordinator authorized the consumer follow ups on this branch, so the PR carries them: the controller catalog copy and report schema copy, the version pins, `catalog.rs`, the runner (`hosts.py`, `series.py`, report format 0.15.0 and every test that pins the version), the example counts in `realism_con23_test.go`, the regenerated `ui/src/api/schema.d.ts`, and the passages of [Realism](Realism.md) and `realism/README.md`.

### Shared page notes

- [Contracts](Contracts.md) is updated (a section for CON-27).
- [Realism](Realism.md) is updated (format 0.15.0, the series of `host_identity`, the rule for two checks of one kind).
- [Testing](Testing.md), [Home](Home.md), and [Implementation Plan](Implementation-Plan.md): none.
