# CON-27 The host identity check in the metric catalog

Contract. Metric catalog `contracts/metrics/catalog.yaml`, the examples `contracts/examples/realism/check-metrics.yaml`, and the mapping rule of `contracts/changes/CON-23.md`. RLM-10 added the `host_identity` check to the realism report (format 0.13.0) after CON-25, so the label `check` of catalog 0.6.0 has no value `host_identity`. A `realism.check_failures` series for the check would break the catalog, and a failing `host_identity` check never reached `RunReport.realism` (RLM-17 leaves its `metrics` empty). CON-23 also left open what `realism.check_failures` means when one document holds two checks of one kind (CTL-42). The owner approved the change as a contract change on 2026-11-11 (comment on issue #480). The change is additive.

Versions.

- The metric catalog moves from 0.6.0 to 0.7.0. It adds one value of an existing label, `host_identity` for `check`, which a reader can see, so it takes a minor number, as CON-23 and CON-25 did. No metric, unit, or other label changes, and every earlier value keeps its place.
- OpenAPI (0.12.0), the agent protocol, the scenario contract, the test configuration, and the module manifest schema stay as they are. `RealismResult` is unchanged.
- The realism report format moves in the package that emits the series (see the follow up). This package does not edit `realism/`.

## Catalog change

`check` gains the value `host_identity`, at the end of its list. The check carries one series, `realism.check_failures` with `check: host_identity`, and no other. Its value is `failures_total` of the check entry: the number of failure sentences, which counts each address conflict, a host count outside the range of the labels, a capture that holds no IP host, and a capture without an Ethernet source address. It is 0 exactly when the check passed, as for every other kind. The series cannot be missing, so a check that failed for a reason that no other series shows still reaches `RunReport.realism`.

| Check kind | Series next to `realism.check_failures` | Value of `realism.check_failures` |
|---|---|---|
| `host_identity` | none | `failures_total` |

The mapping of CON-23 needs no new row. `realism.check_failures` has the target `== 0` and the outcome `passed` of the check whose kind is the label, so the controller builds the `host_identity` entry with the rule that it has for every other kind.

## Two checks of one kind

A realism report holds at most one check of each kind. The failures series carries the kind as its label, and CON-21 makes a metric and its labels unique within one report, so a second check of the same kind would need a second series with the same name and labels, which no document can hold. The rule for `realism.check_failures` is therefore:

- The `passed` of the series is the `passed` of the one check whose kind is the label `check`.
- A document with two checks of one kind is not a valid report. The runner never writes one (it adds one entry for each kind it runs), and a controller that reads one gives the finding that it already gives for a metric and labels pair that repeats, at the pointer of the series of the second check. It does not combine the two outcomes (it neither takes the first, nor requires both to pass), because a combined outcome would hide which check the series speaks for. The report schema cannot state the rule for the kinds without a series (JSON Schema has no uniqueness over a property of the items), so the controller finding is the check, and the schema keeps its rule of exactly one failures series per check.

This holds for every kind that the label names, not only for `host_identity`. A kind that the label does not name carries no series, so two checks of that kind cause no finding. No such kind exists after this change, because the label names all ten kinds of the report.

## Examples

`contracts/examples/realism/check-metrics.yaml` gains the kind `host_identity` with a passing case (`failures_total` 0) and a failing case (`failures_total` 2: one address conflict and a host count outside the range of the labels), in the literal form of the other kinds.

## Compatibility

A client that ignores the value sees no difference. A reader that lists the label values of `check` sees one more. Reports that carry no `host_identity` series stay valid against the catalog. A report of the new format with a `host_identity` check must carry the series (the schema demands it), so a report of format 0.14.0 that holds the check with an empty `metrics` does not validate against the schema of the new format, as a 0.10.0 report did not validate against 0.11.0 (see CON-23).

## Consumers

| Consumer | Package | Uses |
|---|---|---|
| Realism runner | RLM follow up | Emits `realism.check_failures` with `check: host_identity` and the value `failures_total` in `realism/hosts.py`, adds the value to `CATALOG_CHECKS`, and drops the comments that say the catalog lacks it |
| fps-stats tables | `engine/fps-stats` | Mirrors the label values in `catalog.rs`: `host_identity` is appended to the values of `check` in `LABEL_VALUES`. No engine code records the gauge |
| Controller catalog copy | CTL | `controller/internal/metrics/catalog.yaml` is a byte copy of the catalog |
| Controller report mapping and example count test | CTL-42, `realism_con23_test.go` | Reads the new case of `check-metrics.yaml` as a literal, counts the new kind and its two cases, and applies the rule for two checks of one kind. The generic rule of `realism.check_failures` already covers the new kind |
| Python SDK, Web UI, CLI | none | `RealismResult` is unchanged. The views list the new entry without code |

## Follow up outside the catalog paths

The coordinator applies these on the branch, and this package does not edit them. Expect the jobs for each file to be red until then.

- `controller/internal/metrics/catalog.yaml`: `cp contracts/metrics/catalog.yaml controller/internal/metrics/catalog.yaml`.
- `tools/contracts/test_contracts.py`, `test_catalog_is_0_3_0_with_property_both`: `"0.6.0"` becomes `"0.7.0"`.
- `realism/tests/test_mix.py`, `test_the_series_validate_against_catalog_0_4_0`: `"0.6.0"` becomes `"0.7.0"`.
- `engine/fps-stats/src/catalog.rs`: append `"host_identity"` to the values of `check` in `LABEL_VALUES`. The metric ids and `CATALOG.len()` stay.
- `realism/` (an RLM package): `hosts.py` returns the series in place of `[]`, `series.py` adds the value to `CATALOG_CHECKS`, the report schema stops excluding the check from the failures rule and no longer forces its `metrics` to be empty, and the format moves to the next free number.
- `controller/internal/report/schema/realism-report.schema.json`: a byte copy of the runner schema.
- `controller/internal/report/realism_con23_test.go`: the counts of the examples (kinds and cases).
- `ui/src/api/schema.d.ts`: regenerated (`npm run gen:api` in `ui`) when a description carries the label values.
- `docs/wiki/Realism.md` and `realism/README.md`: drop the passages that say the check carries no series.
