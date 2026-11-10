# CON-23 Catalog metrics for every realism check

Contract. Metric catalog `contracts/metrics/catalog.yaml`, the text of the controller OpenAPI document `contracts/openapi/fps-api-v1.yaml`, and the mapping rule of `contracts/changes/CON-22.md`. The change takes the finding of CON-22: of the eight check kinds of the realism report only `conformance`, `temporal`, and the classifier series of `mix_accuracy` carry metrics, so a failing `parser_errors`, `encryption`, `content`, `session_structure`, or `transport_behavior` check adds no `RealismResult` and `RunReport.realism` does not show it. The owner approved the change on 2026-11-10 and chose catalog metrics per check kind over a separate verdict list on `RunReport`. Every change is additive.

Versions.

- The metric catalog moves from 0.4.0 to 0.5.0. It adds seven metrics, two labels, and eight values of the label `property`, which a reader can see, so the change takes a minor number, as CON-20 did for the label `source`.
- The OpenAPI document stays at 0.10.0. Only the text of two descriptions changes (`attachRealismReport` and `RunReport.realism`), as the agent protocol stayed at 0.2.3 when only a comment changed (CON-16). No shape changes, so `api.APIVersion` stays. The generated Go code, the Python SDK, and `ui/src/api/schema.d.ts` carry the descriptions, so they are regenerated (see the follow up).
- The realism report format moves from 0.10.0 to 0.11.0 in the RLM package that emits the metrics. The schema lives under `realism/` and this package does not edit it. The follow up gives its exact change.
- The agent protocol (0.2.3), the scenario contract (v0.6), the test configuration (v0.3), and the module manifest schema (v0.4) stay as they are.

## Catalog additions

The seven metrics are gauges at the level `application` and in the view `realism`. They sit at the end of the file, so the position of every earlier metric, which the engine table (`fps-stats`) uses as its identifier, does not move.

| Metric | Unit | Labels | Value |
|---|---|---|---|
| `realism.check_failures` | `events` | `check` | The number of reasons for which the check failed. It is 0 exactly when the check passed. Every check of the report carries one series |
| `realism.parser_errors` | `events` | `application`, `parser` | Findings of the reference parser in the sessions of the application (`by_parser[parser].parser_errors` of its entry in `applications`). The application `all` totals every finding of the parser, which is the per application values plus `unlabelled_errors[parser]` and `unattributed_errors[parser]` |
| `realism.decrypted_ratio` | `ratio` | none | `decrypted_sessions` over `sampled_sessions`, not rounded |
| `realism.ja4_match_ratio` | `ratio` | none | `ja4_matched_sessions` over `sampled_sessions`, not rounded |
| `realism.duplicate_ratio` | `ratio` | `application` | `duplicate_ratio` of the entry of `applications` |
| `realism.structure_failures` | `events` | `scenario` | The rules that the instances of the scenario break: the failure sentences of that scenario. Their sum over the scenarios is `failures_total`. Two scenarios of one name are one series with the summed value |
| `realism.transport_deviations` | `packets` | `property`, `side` | The packets of the role whose field differs from the profile, summed over the hosts: `mismatched` of the entries of `deviations` with `enforced` true, where `property` is the `field` and `side` is `client` for the role `syn` and `server` for `syn-ack` |

New labels. `check` (the check kind: `conformance`, `parser_errors`, `encryption`, `content`, `session_structure`, `temporal`, `mix_accuracy`, `transport_behavior`) and `parser` (`tshark`, `zeek`, `stub`). A new check kind therefore needs a catalog change, which is where its metrics come from.

Extended labels. `property` gains `ttl`, `df`, `mss`, `window_scale`, `sack`, `timestamps`, `options`, and `syn_window`, the fields that the transport behavior check compares, and only `realism.transport_deviations` uses them. The text of `side` and `scenario` says how the new metrics use them. Every earlier value and metric stays.

Reuse. `application` (with the value `all` of CON-19), `scenario`, `side`, and `property` are the labels of the earlier realism metrics. The two new labels are the only ones that no earlier metric had a use for.

## Series of each check kind

Every check carries `realism.check_failures` with its own kind as the label `check`. The other series are what the check reports about the thing that it judged. A series that no data supports is absent, and the failures series then shows the failure alone.

| Check kind | Series next to `realism.check_failures` | Value of `realism.check_failures` |
|---|---|---|
| `conformance` | `realism.conformance_ratio` for each scored application | The applications whose `passed` is false, plus 1 when no application was scored |
| `temporal` | `realism.ks_statistic` and `realism.ks_p_value` of the comparisons with a catalog label | The length of `failures`: a comparison that is not `pass`, and 1 when the labels name no target |
| `parser_errors` | `realism.parser_errors` for each application and parser that ran, and for the application `all` of each parser | `errors_total`, plus 1 when no parser ran |
| `encryption` | `realism.decrypted_ratio` and `realism.ja4_match_ratio`, only when `sampled_sessions` is above 0 | The length of `failures` |
| `content` | `realism.duplicate_ratio` for each entry of `applications` | The length of `failures`, which also counts a size expectation that failed and an empty analysis |
| `session_structure` | `realism.structure_failures` for each entry of `scenarios` | `failures_total` |
| `mix_accuracy` | `mix.share_pct`, `mix.abs_error_pp`, and `mix.max_abs_error_pp` with `source: classifier` (CON-20), when `metrics_basis` is not null | `failures_total` |
| `transport_behavior` | `realism.transport_deviations` for each enforced field and role in which at least one packet of a profiled host was compared | `failures_total` |

The failures series is the one that cannot be missing: a check whose only reason is that no parser ran, that no TLS session was sampled, that no TCP handshake was profiled, that the counters of the run disagree with the mix, or that nDPI named sessions as no application has no other series, and `realism.check_failures` carries it into `RunReport.realism`.

## Mapping to `RealismResult`

This table replaces the table of CON-22 and keeps its two rows. A controller builds one entry for each metric of each check in the order of the document, as CON-22 says, and the fields `metric`, `labels`, and `observed` stay the name, the labels, and the value. A number in a `target` is written in the shortest decimal form that reads back to the same value (`0.99`, `2`, `60`), and `passed` follows the last column.

| Metric | `target` | `passed` |
|---|---|---|
| `realism.conformance_ratio` | `>= ` and the `threshold` of the check | The `passed` of the entry of `applications` whose `application` is the label (CON-22) |
| `realism.ks_p_value` and `realism.ks_statistic` | `p value >= ` and the `alpha` of the comparison | True when the `status` of that comparison is `pass` (CON-22) |
| `realism.check_failures` | `== 0` | The `passed` of the check whose kind is the label `check` |
| `realism.parser_errors` | `== 0` | The value is 0 |
| `realism.decrypted_ratio` | `>= ` and `min_decrypted` | The value is at least `min_decrypted` |
| `realism.ja4_match_ratio` | `>= 1` | The value is 1 |
| `realism.duplicate_ratio` | `<= ` and `max_duplicate_ratio` | The `passed` of the entry of `applications` whose `application` is the label |
| `realism.structure_failures` | `== 0` | The value is 0 |
| `realism.transport_deviations` | `== 0` | The value is 0 |
| `mix.share_pct` | `within ` and `tolerance_points`, ` points of `, and the `configured_pct` of the row | The `ndpi_passed` of the row of `applications` of the basis `metrics_basis` whose `application` is the label |
| `mix.abs_error_pp` | `<= ` and `tolerance_points` | The same `ndpi_passed` |
| `mix.max_abs_error_pp` | `<= ` and `tolerance_points` | True when `ndpi_passed` holds in every row of `applications` of the basis `metrics_basis` |
| Any other metric | `check passed` | The `passed` of its check (CON-22) |

Every check kind now has a metric with a target that shows the outcome, and the failures series settles the cases that the others cannot show. `property` stays wording for an operator and takes no part in the pairing. `contracts/examples/realism/check-metrics.yaml` holds the literal results, with one passing and one failing case for every kind (and a second failing case for `parser_errors` without a parser), so a controller test and a runner test can read them as golden values.

The mix rows judge two sources. A run that differs in its own metrics (`metrics_passed`), or whose nDPI sessions fall into no application (the row `unmatched`), fails the check without moving a classifier series, and `realism.check_failures` shows it. A later report may carry series with `source: metrics`. They map by the last row of the table until a package gives them a rule.

## Compatibility

A client that ignores the new metrics sees no difference: every earlier metric, label, and value stays, the new ones sit after them, and no OpenAPI shape changes. `RunReport.realism` gains entries for the checks that carried no metric before, and an entry for `realism.check_failures` next to each check. A reader that pairs entries of two runs by `metric` and `labels` pairs them as before.

The report schema is the one place where the change is not additive for a document: a report of format 0.10.0 has an empty `metrics` array in six check kinds and lacks `realism.check_failures`, so it breaks the schema of 0.11.0. No consumer that rejects it ships yet (the controller handler of CTL-41 does not exist), and the runner and the schema change in one package. A schema that must also accept 0.10.0 documents wraps the three added `allOf` entries of `$defs.check` in an `if` on `version`, which the follow up does not do.

`python3 tools/contracts/validate.py` passes with the new catalog. `tools/contracts/test_contracts.py` pins the catalog version, so it fails on one test until the coordinator applies the follow up. The validator does not read `contracts/examples/realism/`, so the first controller and runner tests that load it are its check. I compared every case with the catalog (names, labels, label values, targets, and `passed`) and with the schema patch below in throwaway scripts.

## Consumers

| Consumer | Package | Uses |
|---|---|---|
| Realism runner | RLM follow up | Emits the series of the table above for every check, applies the schema change, and raises the report format to 0.11.0 |
| Controller | CTL-41 | Embeds the new schema copy, builds the entries by the mapping table, and reads the examples as test literals |
| Engine tables | `engine/fps-stats` | Mirrors the seven metrics and the labels in `catalog.rs`. No engine code records these gauges |
| Python SDK, Web UI, CLI | none | `RealismResult` is unchanged. The views list the new entries without code. The regenerated descriptions follow |

## Follow up outside the catalog paths

The coordinator applies these on the branch, and this package does not edit them. Expect the jobs for each file to be red until then.

- `controller/internal/metrics/catalog.yaml`: a byte copy of the catalog. `cp contracts/metrics/catalog.yaml controller/internal/metrics/catalog.yaml` (`test_controller_copies_are_byte_identical` fails until then).
- `tools/contracts/test_contracts.py`, `test_catalog_is_0_3_0_with_property_both`: `catalog_doc["version"] == "0.4.0"` becomes `"0.5.0"`. The assertion `values[:5] == [...]` stays true, because the earlier values keep their place.
- `realism/tests/test_mix.py`, `test_the_series_validate_against_catalog_0_4_0`: `assert catalog["version"] == "0.4.0"` becomes `"0.5.0"` (the `rlm` workflow runs on a change of the catalog).
- `engine/fps-stats/src/catalog.rs`:
  - After `host.active` append seven `MetricDef` entries, all `MetricKind::Gauge`, in this order and with these units and labels: `realism.check_failures` (`events`, `["check"]`), `realism.parser_errors` (`events`, `["application", "parser"]`), `realism.decrypted_ratio` (`ratio`, `[]`), `realism.ja4_match_ratio` (`ratio`, `[]`), `realism.duplicate_ratio` (`ratio`, `["application"]`), `realism.structure_failures` (`events`, `["scenario"]`), and `realism.transport_deviations` (`packets`, `["property", "side"]`).
  - In the module `metric`, add `REALISM_CHECK_FAILURES = MetricId(48)`, `REALISM_PARSER_ERRORS = MetricId(49)`, `REALISM_DECRYPTED_RATIO = MetricId(50)`, `REALISM_JA4_MATCH_RATIO = MetricId(51)`, `REALISM_DUPLICATE_RATIO = MetricId(52)`, `REALISM_STRUCTURE_FAILURES = MetricId(53)`, and `REALISM_TRANSPORT_DEVIATIONS = MetricId(54)`, each with its doc comment (`` `realism.check_failures`, a gauge in `events`. ``).
  - In `LABEL_VALUES`, append `ttl`, `df`, `mss`, `window_scale`, `sack`, `timestamps`, `options`, and `syn_window` to `property`, and add `("check", &["conformance", "parser_errors", "encryption", "content", "session_structure", "temporal", "mix_accuracy", "transport_behavior"])` and `("parser", &["tshark", "zeek", "stub"])`.
  - In `LABEL_NAMES`, insert `"check"` and `"parser"` after `"property"` and before `"alert"` (catalog order).
- `engine/fps-stats/tests/catalog.rs`: `assert_eq!(CATALOG.len(), 48)` becomes `55`.
- `controller/internal/api/api.gen.go` (`go generate ./internal/api` in `controller`), `sdk/python/fixingpoint/_generated.py` (`python3 sdk/python/scripts/generate_client.py`), and `ui/src/api/schema.d.ts` (`npm run gen:api` in `ui`): regenerated for the two descriptions. `TestGeneratedCodeIsCurrent` fails until the Go file is.
- `realism/schema/realism-report.schema.json` (the RLM package, in the same change as the emitters, because the realism tests validate every report against the schema): apply `contracts/changes/CON-23-realism-report-schema.patch` with `patch -p1`. It adds
  - the definitions `failures_metric`, `parser_errors_metric`, `encryption_metric`, `content_metric`, `session_structure_metric`, `transport_behavior_metric`, and (moved from the check definitions) `conformance_metric`, `temporal_metric`, and `mix_metric` under `$defs`,
  - for every check definition a `metrics` property whose items are the metric definition of its kind or `failures_metric`, and which must contain the `realism.check_failures` series with the label `check` equal to the kind (this replaces `maxItems: 0` in the five kinds that had no metric),
  - in `$defs.check.allOf` two entries: exactly one `realism.check_failures` per check, and its value is 0 when `passed` is true and at least 1 otherwise.

  The report `version` moves to 0.11.0 in the runner (`realism/report.py`) and in the page `docs/wiki/Realism.md`. The runner also drops the TODO comments of issue #89 in `parser_errors.py`, `encryption.py`, `content.py`, and `structure.py`, and `transport.py` returns its series in place of `[]`.
- `docs/wiki/Realism.md` (delivery and the series of each check) and `docs/wiki/Controller.md` (the mapping table) are for the owners of those pages, with the packages above.
