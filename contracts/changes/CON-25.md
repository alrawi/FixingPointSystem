# CON-25 The messages check in the metric catalog

Contract. Metric catalog `contracts/metrics/catalog.yaml` and the examples `contracts/examples/realism/check-metrics.yaml`. RLM-14 added the `messages` check to the realism report (format 0.11.0) after CON-23 listed the eight check kinds, so the label `check` of catalog 0.5.0 has no value `messages`. A `realism.check_failures` series for the check would break the catalog, and a failing `messages` check never reached `RunReport.realism` (RLM-17 leaves its `metrics` empty). The owner approved the change as a contract change on 2026-11-11 (comment on issue #458). The change is additive.

Versions.

- The metric catalog moves from 0.5.0 to 0.6.0. It adds one value of an existing label, `messages` for `check`, which a reader can see, so it takes a minor number, as CON-23 did. No metric, unit, or other label changes, and every earlier value keeps its place.
- OpenAPI (0.10.0), the agent protocol, the scenario contract, the test configuration, and the module manifest schema stay as they are. `RealismResult` is unchanged.
- The realism report format moves in the RLM package that emits the series (see the follow up). This package does not edit `realism/`.

## Catalog change

`check` gains the value `messages`, at the end of its list. The check carries one series, `realism.check_failures` with `check: messages`, and no other. Its value is `failures_total` of the check entry: the number of failure sentences, which counts a session outside its range, a rule that selects no TCP session, and a missing tshark. It is 0 exactly when the check passed, as for every other kind. The series cannot be missing, so a check that failed for a reason that no other series shows still reaches `RunReport.realism`.

| Check kind | Series next to `realism.check_failures` | Value of `realism.check_failures` |
|---|---|---|
| `messages` | none | `failures_total` |

The mapping of CON-23 needs no new row. `realism.check_failures` has the target `== 0` and the outcome `passed` of the check whose kind is the label, so the controller builds the `messages` entry with the rule that it has for every other kind.

## Examples

`contracts/examples/realism/check-metrics.yaml` gains the kind `messages` with a passing case (`failures_total` 0) and a failing case (`failures_total` 2: one session without its response and one rule that selects no session), in the literal form of the other kinds.

## Compatibility

A client that ignores the value sees no difference. A reader that lists the label values of `check` sees one more. Reports that carry no `messages` series stay valid against the catalog.

## Consumers

| Consumer | Package | Uses |
|---|---|---|
| Realism runner | RLM follow up | Emits `realism.check_failures` with `check: messages` and the value `failures_total` in `realism/messages.py`, and drops the comment that says the catalog lacks the value |
| fps-stats tables | `engine/fps-stats` | Mirrors the label values in `catalog.rs`: `messages` is appended to the values of `check` in `LABEL_VALUES`. No engine code records the gauge |
| Controller catalog copy | CTL | `controller/internal/metrics/catalog.yaml` is a byte copy of the catalog |
| Controller report mapping | CTL-42 | Reads the new case of `check-metrics.yaml` as a literal. The generic rule of `realism.check_failures` already covers it |
| Python SDK, Web UI, CLI | none | `RealismResult` is unchanged. The views list the new entry without code |

## Follow up outside the catalog paths

The coordinator applies these on the branch, and this package does not edit them. Expect the jobs for each file to be red until then.

- `controller/internal/metrics/catalog.yaml`: `cp contracts/metrics/catalog.yaml controller/internal/metrics/catalog.yaml` (`test_controller_copies_are_byte_identical` fails until then).
- `tools/contracts/test_contracts.py`, `test_catalog_is_0_3_0_with_property_both`: `catalog_doc["version"] == "0.5.0"` becomes `"0.6.0"`.
- `realism/tests/test_mix.py`, `test_the_series_validate_against_catalog_0_4_0`: `assert catalog["version"] == "0.5.0"` becomes `"0.6.0"`.
- `engine/fps-stats/src/catalog.rs`: append `"messages"` to the values of `check` in `LABEL_VALUES`. The metric ids and `CATALOG.len()` stay.
- `realism/` (an RLM package): `messages.py` returns the series in place of `[]`, and the report schema stops excluding the check from the failures rule and no longer forces its `metrics` to be empty. In `realism/tests/test_check_metrics.py` three tests fail until then, and they are the package's own check of this change: `test_the_messages_check_stays_outside_the_failures_rule_until_the_catalog_names_it` (it pins the absence of the value `messages`, so it is replaced by a test that the check carries the series), `test_the_series_of_the_contract_examples_validate_against_the_schema[messages-passing and messages-failing]` (the schema still demands an empty `metrics`), and `test_the_contract_examples_cover_every_kind_with_a_passing_and_a_failing_case` (its list of kinds lacks `messages`). The format moves to 0.13.0 and `docs/wiki/Realism.md` drops the passages that say the check carries no series.
