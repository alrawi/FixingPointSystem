# Handoff CON-02 Counter catalog v0.1

| Field | Value |
|---|---|
| Issue | #18 |
| Branch | `wp/CON-02` |
| Pull request | #64 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-con02-9dd7, lease until 2026-11-02T18:58:01Z |
| Last verified commit | the head of `wp/CON-02` at finish |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Name every counter, gauge, and histogram once in `contracts/metrics/catalog.yaml`, with type, unit, level, labels, and description, so that the agent, the controller, the API, and the UI agree on metric names. A validator rule rejects metric names in example test criteria and in the OpenAPI document that the catalog does not define.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Write the catalog JSON Schema and `contracts/metrics/catalog.yaml` for the four levels of the specification.
- [x] 2. Add catalog checks and the metric reference rule to `tools/contracts/validate.py`, and annotate the OpenAPI run summary with catalog names.
- [x] 3. Add tests, including one that shows an unknown metric fails.
- [x] 4. Update `contracts/README.md` and `contracts/CHANGELOG.md`.
- [x] 5. Ingest into the wiki, append the Log entry, and finish.

## Done and verified

1. The catalog holds 48 metrics at five levels and validates against `contracts/metrics/catalog.schema.json`. Verified by `test_catalog_is_valid_and_covers_four_levels`.
2. The validator checks the catalog (schema, duplicate names, undeclared labels, formula references, collisions with histogram statistics), every criterion metric of every test, and every `x-fps-metric` annotation in the OpenAPI document. Every `RunSummary` property must carry an annotation. Verified by `python3 tools/contracts/validate.py`, which reports 0 errors and 0 warnings.
3. Sixteen new test cases cover the rules. `test_unknown_criteria_metric_fails`, `test_unknown_criteria_metric_fails_the_cli`, and `test_openapi_unknown_metric_fails` show that an unknown metric fails. Verified by `python3 -m pytest -q tools/contracts`, which reports 58 passed.
4. `contracts/README.md` gained a Metric catalog section, and `contracts/CHANGELOG.md` records contract 0.1.1.
5. The wiki page Implementation Plan gained a Counter catalog section, and Log has the ingest entry. Verified by `python3 tools/plan/plan.py lint`.

## Remaining

Owner approval of the contract change. The open questions below may lead to follow up packages.

## Next action

Wait for owner review of pull request #64 and apply requested changes.

## Blockers and open questions

1. The OpenAPI `MetricView` enum has six values (application, scenario, port, tcp, tls, realism), while the catalog has five levels. We left the enum unchanged and did not map views to levels. A later CON package could add the mapping.
2. Criteria carry no label filter, so a criterion always aggregates over all labels. Per application criteria need a schema change to the test configuration.
3. The OpenAPI document changed from 0.1.0 to 0.1.1 to add `x-fps-metric` annotations, although `contracts/openapi/` is outside the package paths. The deliverable requires metric names in the OpenAPI document, and without annotations the rule would have nothing to check. The owner should confirm this choice.
4. The rule that a histogram reference needs a statistic suffix (`p50`, `p90`, `p99`, `p999`, `min`, `max`, `mean`, `count`) is our proposal and needs owner confirmation.

## Decisions taken in this package

- The application and scenario row of the specification splits into two levels. Session, transaction, and byte counters sit at the scenario level with labels `application` and `scenario`, and the application view sums over `scenario`. Mix and realism gauges sit at the application level.
- Derived gauges such as `sessions.failed_ratio` and `tls.resumption_ratio` carry a `formula`, and the validator checks that the formula refers only to catalog counters and gauges.
- Histograms use microseconds (`us`) or bytes, which matches HDR histogram practice.
- Labels are declared once under `labels`, with closed value sets where the domain is closed, for example `reason` and `side`.
- The validator loads the repository catalog once through `default_catalog()` when a caller does not pass one, so existing callers of `check_test_config` and `check_openapi` keep working.
- Prometheus names map to `fps_` plus the catalog name with dots replaced by underscores, with `_total` on counters. The README records the mapping, and OPS or CTL packages implement it.

## Pitfalls

- `ruff check` with default rules reports nine findings in `tools/contracts`, all of which predate this package. CI does not run ruff on these tools.

## Verify the current state

```bash
python3 -m pip install -r tools/contracts/requirements.txt
python3 tools/contracts/validate.py     # expect 0 errors, 0 warnings
python3 -m pytest -q tools/contracts     # expect 58 passed
python3 tools/plan/plan.py lint          # expect ok
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T17:28:01Z | claude-con02-9dd7 | claim | none | Fresh claim, no earlier cards |
| 2026-11-02T17:50:00Z | claude-con02-9dd7 | finish | see branch head | All five plan steps done, ready for review |
