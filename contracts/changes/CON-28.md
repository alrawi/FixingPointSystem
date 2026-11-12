# CON-28 Packet gap series in the metric catalog

Contract. Metric catalog `contracts/metrics/catalog.yaml`, the examples `contracts/examples/realism/check-metrics.yaml`, and the mapping rule of `contracts/changes/CON-23.md`. RLM-15 added the property `packet_gap` to the temporal check of the realism report (format 0.16.0), the gaps in seconds between consecutive packets with payload inside one stream of a session. The label `property` of `realism.ks_statistic` and `realism.ks_p_value` has no value for it, so the runner could not emit a KS series for packet gaps, and only `realism.check_failures` showed a failing comparison in `RunReport.realism`. The owner approved the change as a contract change on 2026-11-12 (comment on issue #496). The change is additive.

## Versions

- The metric catalog moves from 0.7.0 to 0.8.0. It adds three values of an existing label, which a reader can see, so it takes a minor number, as CON-23, CON-25, and CON-27 did. No metric, unit, or other label changes, and every earlier value keeps its place.
- OpenAPI (0.12.0), the agent protocol, the scenario contract, the test configuration, and the module manifest schema stay as they are. `RealismResult` is unchanged.
- The realism report format moves in this package, because the runner change belongs to the same pull request (see the consumers). The follow up list below names it.

## Catalog change

`property` gains the values `packet_gap`, `request_gap`, and `response_gap`, at the end of its list, in this order. They follow the pattern of the sizes (`request_bytes`, `response_bytes`, and `both`), which fold the direction into the value:

| Value | What the comparison holds | Comparison of the temporal check |
|---|---|---|
| `request_gap` | The gaps of the client to server streams of the application | `property: packet_gap`, `direction: client` |
| `response_gap` | The gaps of the server to client streams | `property: packet_gap`, `direction: server` |
| `packet_gap` | The gaps of the streams of both directions pooled | `property: packet_gap`, `direction: both` |

A stream is one direction of one session, and a gap never spans two streams (RLM-15). Only `realism.ks_statistic` and `realism.ks_p_value` use the values, with the labels `application` and `property` as before. The application value `all` is not used, as for the other properties (a comparison over every session of the capture has no application label, and the runner leaves it in `comparisons`).

A comparison of a paced target (a `constant` target with a list of packet times, RLM-15) has no KS statistic. Its `statistic` is the share of gaps outside the tolerance, and its `p_value` is 0 or 1 by the verdict. Neither is what the two metrics describe, so a paced comparison carries no series, and `realism.check_failures` shows its failure, as it shows the comparisons without a label today. The `constant` target of a packet gap is always paced, even with one packet time, so the series exist for the parametric targets (`exponential`, `uniform`, `lognormal`, `pareto`) and the `empirical` targets of packet gaps.

`payload_size` is not a value. The issue text listed it next to `packet_gap`, but the sizes of the payloads already have their values: `request_bytes` for the client direction, `response_bytes` for the server direction (CON-22 names them as the `payload_size` comparisons of those directions), and `both` for the two directions pooled (already in the catalog). A second value for the same series would give two names to one comparison, and a report would hold the same series twice or the controller would have to choose. The sizes of both directions are a runner matter and not part of this change: the catalog value `both` exists, and the runner may emit it in a later RLM change without a contract change.

## Mapping to `RealismResult`

The mapping of CON-23 needs no new row. The rows of `realism.ks_p_value` and `realism.ks_statistic` apply: the `target` is `p value >= ` and the `alpha` of the comparison, and `passed` is true when the `status` of the comparison is `pass`. The controller finds the comparison of a series by the label `application` and the label `property`, which for the new values means the comparison of the property `packet_gap` with the direction of the table above. A comparison of two targets with the same application and direction does not exist (the labels file refuses it), so the pair is unique.

## Examples

`contracts/examples/realism/check-metrics.yaml` gains two cases for the kind `temporal`, in the literal form of the other cases: `passing packet gaps` (the series `request_gap` and `packet_gap` of the application `rtp`, passing) and `failing packet gaps` (the series `response_gap`, failing, with the series `realism.check_failures` of value 1).

## Compatibility

A client that ignores the values sees no difference. A reader that lists the values of `property` sees three more. A report of the earlier formats stays valid against the catalog. A report of the new format may carry the series (the schema allows them), and a report without them is still valid, because the series exist only for the comparisons that a labels file asks for.

## Consumers

| Consumer | Package | Uses |
|---|---|---|
| Realism runner | RLM, in this pull request | `realism/temporal.py` emits the KS series of the comparisons of the property `packet_gap` with the values above, the report schema accepts them, and the report format moves to the next free number after 0.16.0 |
| fps-stats tables | `engine/fps-stats` | Mirrors the label values in `catalog.rs`: the three values are appended to the values of `property` in `LABEL_VALUES`. No engine code records the gauge |
| Controller catalog copy | CTL | `controller/internal/metrics/catalog.yaml` is a byte copy of the catalog |
| Controller report mapping | CTL-42, `realism.go` | `comparisonProperty` returns the new values for the comparisons of the property `packet_gap` by direction, so the rule of CON-23 finds the comparison |
| Controller example count test | `realism_con23_test.go` | Reads the new cases of `check-metrics.yaml` as literals and counts them |
| Python SDK, Web UI, CLI | none | `RealismResult` is unchanged. The views list the new entries without code |

## Follow up in this pull request, outside the catalog paths

The coordinator authorized these on the branch of this package, so that the pull request is green in one round.

- `controller/internal/metrics/catalog.yaml`: `cp contracts/metrics/catalog.yaml controller/internal/metrics/catalog.yaml`.
- `tools/contracts/test_contracts.py`, `test_catalog_is_0_3_0_with_property_both`: `"0.7.0"` becomes `"0.8.0"`.
- `realism/tests/test_mix.py`, `test_the_series_validate_against_catalog_0_4_0`: `"0.7.0"` becomes `"0.8.0"`.
- `engine/fps-stats/src/catalog.rs`: append `"packet_gap"`, `"request_gap"`, and `"response_gap"` to the values of `property` in `LABEL_VALUES`.
- `realism/` (an RLM change): `temporal.py` emits the series, the report schema accepts the new values, `series.py` and the tests name the catalog version, and the format moves to the next free number.
- `controller/internal/report/schema/realism-report.schema.json`: a byte copy of the runner schema.
- `controller/internal/report/realism.go` and `realism_con23_test.go`: the mapping of the new values and the counts of the examples.
- `ui/src/api/schema.d.ts`: regenerated, because a description of the report schema carries the label values.
- `docs/wiki/Realism.md` and `realism/README.md`: drop the passages that say packet gaps have no series.

Module authors write no fixture labels differently after this change: the `temporal.packets` section of the labels file (RLM-15) is unchanged, so `docs/wiki/Module-Development.md` needs no new text.
