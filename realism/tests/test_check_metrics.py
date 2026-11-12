# SPDX-License-Identifier: GPL-3.0-only
"""The metric series of every check of the realism report (CON-23 and CON-25, report format 0.14.0).

Every check kind of the metric catalog carries ``realism.check_failures``, and most carry a
series of their own. Each case below builds a small capture or a committed fixture, runs one
check, and compares the whole ``metrics`` array with a literal, once for a passing and once
for a failing check. The entry then has to validate against the report schema, and the
examples of ``contracts/examples/realism/check-metrics.yaml`` have to validate against it too.
The ``messages`` check carries only the failures series since CON-25 (catalog 0.6.0, report
format 0.14.0), and its value is ``failures_total``. The ``host_identity`` check is the
exception: the catalog has no value ``host_identity``, so it carries no series yet.
"""

from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest
import yaml

from realism import conformance, messages, parser_errors, series, structure
from realism.capture import summarize
from realism.classifier import StubClassifier
from realism.encryption import StubTlsProbe, TlsObservation
from realism.encryption import check as encryption_check
from realism.labels import parse
from realism.parsers import KIND_MALFORMED, Finding, StubParser
from realism.report import REPORT_VERSION, SCHEMA_PATH, score
from realism.tests import pcapgen
from realism.tests import test_content as content_tests
from realism.tests import test_encryption as encryption_tests
from realism.tests import test_messages as messages_tests
from realism.tests import test_mix as mix_tests
from realism.tests import test_structure as structure_tests
from realism.tests import test_temporal as temporal_tests
from realism.tests import test_transport as transport_tests
from realism.tests.conftest import LABELS_HTTP_DNS
from realism.tests.test_ja4 import CHROME_JA4

REPO = Path(__file__).resolve().parents[2]
EXAMPLES = REPO / "contracts" / "examples" / "realism" / "check-metrics.yaml"
SCHEMA = json.loads(SCHEMA_PATH.read_text())

Series = list[dict[str, Any]]


def _failures(check: str, value: int) -> dict[str, Any]:
    return {"name": "realism.check_failures", "labels": {"check": check}, "value": value}


def _validate_entry(entry: dict[str, Any]) -> None:
    """Validate one check entry against the schema of the report."""
    jsonschema.Draft202012Validator({"$ref": "#/$defs/check", "$defs": SCHEMA["$defs"]}).validate(
        entry
    )


def _assert_case(entry: dict[str, Any], passed: bool, expected: Series) -> None:
    assert entry["passed"] is passed
    assert entry["metrics"] == expected
    failures = [m for m in entry["metrics"] if m["name"] == "realism.check_failures"]
    assert len(failures) == 1
    assert (failures[0]["value"] == 0) is passed
    _validate_entry(entry)


# --- conformance ----------------------------------------------------------------------------


def _conformance(tmp_path: Path, dns: str | None) -> dict[str, Any]:
    pcap = pcapgen.mixed_capture(http_sessions=3, dns_lookups=3).write_pcap(tmp_path / "c.pcap")
    stub = StubClassifier.by_server_port(
        {80: "HTTP", 53: dns} if dns else {80: "HTTP"}, name="ndpi"
    )
    labels = parse(LABELS_HTTP_DNS)
    result = conformance.check(pcap, summarize(pcap).sessions, labels, stub, 0.99)
    return result.to_dict()


def test_conformance_passing(tmp_path: Path) -> None:
    _assert_case(
        _conformance(tmp_path, "DNS"),
        True,
        [
            {
                "name": "realism.conformance_ratio",
                "labels": {"application": "dns", "classifier": "ndpi"},
                "value": 1.0,
            },
            {
                "name": "realism.conformance_ratio",
                "labels": {"application": "http1", "classifier": "ndpi"},
                "value": 1.0,
            },
            _failures("conformance", 0),
        ],
    )


def test_conformance_failing_counts_the_applications_below_the_threshold(tmp_path: Path) -> None:
    _assert_case(
        _conformance(tmp_path, None),
        False,
        [
            {
                "name": "realism.conformance_ratio",
                "labels": {"application": "dns", "classifier": "ndpi"},
                "value": 0.0,
            },
            {
                "name": "realism.conformance_ratio",
                "labels": {"application": "http1", "classifier": "ndpi"},
                "value": 1.0,
            },
            _failures("conformance", 1),
        ],
    )


def test_conformance_failing_without_a_scored_application_counts_one(tmp_path: Path) -> None:
    pcap = pcapgen.mixed_capture().write_pcap(tmp_path / "c.pcap")
    labels = parse({"labels": "fps-realism-labels", "version": 1, "sessions": []})
    stub = StubClassifier.by_server_port({80: "HTTP"}, name="ndpi")
    entry = conformance.check(pcap, summarize(pcap).sessions, labels, stub, 0.99).to_dict()
    _assert_case(entry, False, [_failures("conformance", 1)])


# --- parser_errors --------------------------------------------------------------------------

DNS_CLIENT = ("10.0.0.10", 50100)
DNS_SERVER = ("10.0.1.53", 53)


def _parser_series(app: str, parser: str, value: int) -> dict[str, Any]:
    return {
        "name": "realism.parser_errors",
        "labels": {"application": app, "parser": parser},
        "value": value,
    }


def _parser_errors(tmp_path: Path, findings: list[Finding], **kwargs: Any) -> dict[str, Any]:
    cap = pcapgen.mixed_capture(http_sessions=2, dns_lookups=2)
    cap.dns_malformed_answer(DNS_CLIENT, DNS_SERVER, 0x2222)
    pcap = cap.write_pcap(tmp_path / "p.pcap")
    parsers = [] if kwargs.get("none") else [StubParser(findings, name="tshark")]
    sessions = summarize(pcap).sessions
    return parser_errors.check(pcap, sessions, parse(LABELS_HTTP_DNS), parsers).to_dict()


def test_parser_errors_passing(tmp_path: Path) -> None:
    _assert_case(
        _parser_errors(tmp_path, []),
        True,
        [
            _parser_series("dns", "tshark", 0),
            _parser_series("http1", "tshark", 0),
            _parser_series("all", "tshark", 0),
            _failures("parser_errors", 0),
        ],
    )


def test_parser_errors_failing_totals_every_finding_of_the_parser(tmp_path: Path) -> None:
    in_dns = Finding(
        parser="tshark",
        kind=KIND_MALFORMED,
        name="dns",
        message="Malformed Packet",
        frame=26,
        timestamp=1.0,
        transport="udp",
        src_ip=DNS_SERVER[0],
        src_port=DNS_SERVER[1],
        dst_ip=DNS_CLIENT[0],
        dst_port=DNS_CLIENT[1],
    )
    nowhere = Finding(parser="tshark", kind=KIND_MALFORMED, name="ip", message="Bad", frame=1)
    entry = _parser_errors(tmp_path, [in_dns, in_dns, nowhere])
    # Two findings in dns sessions and one that no session owns: errors_total is 3.
    assert entry["errors_total"] == 3
    _assert_case(
        entry,
        False,
        [
            _parser_series("dns", "tshark", 2),
            _parser_series("http1", "tshark", 0),
            _parser_series("all", "tshark", 3),
            _failures("parser_errors", 3),
        ],
    )


def test_parser_errors_failing_without_a_parser_carries_only_the_failures_series(
    tmp_path: Path,
) -> None:
    _assert_case(_parser_errors(tmp_path, [], none=True), False, [_failures("parser_errors", 1)])


# --- encryption -----------------------------------------------------------------------------


def _encryption(tmp_path: Path, **observation: Any) -> dict[str, Any]:
    pcap = encryption_tests._stub_capture(tmp_path)
    obs = encryption_tests._observation(pcap, **observation)
    labels = encryption_tests._labels()
    sessions = summarize(pcap).sessions
    return encryption_check(pcap, sessions, labels, StubTlsProbe(obs)).to_dict()


def test_encryption_passing(tmp_path: Path) -> None:
    entry = _encryption(tmp_path, tshark_ja4=CHROME_JA4)
    _assert_case(
        entry,
        True,
        [
            {"name": "realism.decrypted_ratio", "labels": {}, "value": 1.0},
            {"name": "realism.ja4_match_ratio", "labels": {}, "value": 1.0},
            _failures("encryption", 0),
        ],
    )


def test_encryption_failing_gives_the_unrounded_shares(tmp_path: Path) -> None:
    # No session shows Finished messages from both sides, so none decrypts.
    entry = _encryption(tmp_path, finished=0, tshark_ja4=CHROME_JA4)
    assert entry["sampled_sessions"] == 3 and entry["decrypted_sessions"] == 0
    _assert_case(
        entry,
        False,
        [
            {"name": "realism.decrypted_ratio", "labels": {}, "value": 0.0},
            {"name": "realism.ja4_match_ratio", "labels": {}, "value": 1.0},
            _failures("encryption", len(entry["failures"])),
        ],
    )


def test_encryption_failing_with_a_mismatched_profile(tmp_path: Path) -> None:
    pcap = encryption_tests._stub_capture(tmp_path, sessions=2)
    labels = encryption_tests._labels(
        profiles={"chrome": {"ja4": "t13d1517h2_8daaf6152771_e5627efa2ab1"}}
    )
    obs = encryption_tests._observation(pcap, sessions=2)
    entry = encryption_check(pcap, summarize(pcap).sessions, labels, StubTlsProbe(obs)).to_dict()
    _assert_case(
        entry,
        False,
        [
            {"name": "realism.decrypted_ratio", "labels": {}, "value": 1.0},
            {"name": "realism.ja4_match_ratio", "labels": {}, "value": 0.0},
            _failures("encryption", 2),
        ],
    )


def test_encryption_failing_without_a_sampled_session_has_no_ratio(tmp_path: Path) -> None:
    pcap = pcapgen.Capture().write_pcap(tmp_path / "empty.pcap")
    labels = encryption_tests._labels()
    obs = TlsObservation(tool="stub", version="stub")
    entry = encryption_check(pcap, [], labels, StubTlsProbe(obs)).to_dict()
    assert entry["sampled_sessions"] == 0
    _assert_case(entry, False, [_failures("encryption", len(entry["failures"]))])


# --- content --------------------------------------------------------------------------------


def test_content_passing(tmp_path: Path) -> None:
    entry = content_tests._run(tmp_path, content_tests._varied(40)).to_dict()
    _assert_case(
        entry,
        True,
        [
            {"name": "realism.duplicate_ratio", "labels": {"application": "http1"}, "value": 0.0},
            _failures("content", 0),
        ],
    )


def test_content_failing_gives_the_duplicate_ratio_of_the_application(tmp_path: Path) -> None:
    # Ten sessions send one request and receive one response: 20 units, 2 distinct.
    entry = content_tests._run(tmp_path, content_tests._repeated(10)).to_dict()
    _assert_case(
        entry,
        False,
        [
            {"name": "realism.duplicate_ratio", "labels": {"application": "http1"}, "value": 0.9},
            _failures("content", 1),
        ],
    )


def test_content_failing_without_a_payload_carries_only_the_failures_series(
    tmp_path: Path,
) -> None:
    entry = content_tests._run(tmp_path, pcapgen.Capture()).to_dict()
    _assert_case(entry, False, [_failures("content", len(entry["failures"]))])


# --- session_structure ----------------------------------------------------------------------


def _structure(tmp_path: Path, cap: pcapgen.Capture) -> dict[str, Any]:
    return structure_tests._check(tmp_path, cap).to_dict()


def test_session_structure_passing(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    for i in range(3):
        structure_tests._browse(cap, client=f"10.0.0.{10 + i}", port=40000 + 10 * i)
    _assert_case(
        _structure(tmp_path, cap),
        True,
        [
            {
                "name": "realism.structure_failures",
                "labels": {"scenario": "web_browse"},
                "value": 0,
            },
            _failures("session_structure", 0),
        ],
    )


def test_session_structure_failing_counts_the_broken_rules_of_the_scenario(
    tmp_path: Path,
) -> None:
    # A page fetch without the lookup before it breaks the count of the flow lookup.
    cap = pcapgen.Capture()
    cap.http_get((structure_tests.CLIENT, 40001), structure_tests.WEB)
    _assert_case(
        _structure(tmp_path, cap),
        False,
        [
            {
                "name": "realism.structure_failures",
                "labels": {"scenario": "web_browse"},
                "value": 1,
            },
            _failures("session_structure", 1),
        ],
    )


def test_two_scenarios_of_one_name_are_one_series_with_the_summed_value(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.http_get((structure_tests.CLIENT, 40001), structure_tests.WEB)
    pcap = cap.write_pcap(tmp_path / "c.pcap")
    scenario = str(structure_tests.WEB_BROWSE)
    labels = parse(
        {
            "labels": "fps-realism-labels",
            "version": 1,
            "structure": {"scenarios": [{"file": scenario}, {"file": scenario}]},
        }
    )
    result = structure.check(pcap, summarize(pcap).sessions, labels)
    entry = result.to_dict()
    one = [m for m in entry["metrics"] if m["name"] == "realism.structure_failures"]
    assert len(one) == 1
    assert one[0]["value"] == sum(len(s.failures) for s in result.scenarios)
    assert entry["metrics"][-1] == _failures("session_structure", entry["failures_total"])
    _validate_entry(entry)


# --- temporal -------------------------------------------------------------------------------

SIZES_500 = {
    "sizes": {"applications": {"bulk": {"direction": "server", "dist": "constant", "value": 500}}}
}


def _temporal(tmp_path: Path, size: int) -> dict[str, Any]:
    cap = temporal_tests._bulk([size] * 40)
    return temporal_tests._run(tmp_path, cap, temporal_tests._labels(SIZES_500)).to_dict()


def _ks(name: str, value: float) -> dict[str, Any]:
    labels = {"application": "bulk", "property": "response_bytes"}
    return {"name": name, "labels": labels, "value": value}


def test_temporal_passing(tmp_path: Path) -> None:
    _assert_case(
        _temporal(tmp_path, 500),
        True,
        [
            _ks("realism.ks_statistic", 0.0),
            _ks("realism.ks_p_value", 1.0),
            _failures("temporal", 0),
        ],
    )


def test_temporal_failing_counts_the_comparisons_that_are_not_a_pass(tmp_path: Path) -> None:
    _assert_case(
        _temporal(tmp_path, 700),
        False,
        [
            _ks("realism.ks_statistic", 1.0),
            _ks("realism.ks_p_value", 0.0),
            _failures("temporal", 1),
        ],
    )


def test_temporal_failing_without_a_target_counts_one(tmp_path: Path) -> None:
    labels = temporal_tests._labels(None)
    entry = temporal_tests._run(tmp_path, temporal_tests._bulk([500] * 40), labels).to_dict()
    _assert_case(entry, False, [_failures("temporal", 1)])


def test_temporal_failing_with_too_few_observations_has_no_ks_series(tmp_path: Path) -> None:
    entry = temporal_tests._run(
        tmp_path, temporal_tests._bulk([500] * 10), temporal_tests._labels(SIZES_500)
    ).to_dict()
    _assert_case(entry, False, [_failures("temporal", 1)])


# --- mix_accuracy ---------------------------------------------------------------------------

CLASSIFIER = {"source": "classifier", "classifier": "ndpi"}


def _mix(name: str, value: float, **labels: str) -> dict[str, Any]:
    return {"name": name, "labels": {**labels, **CLASSIFIER}, "value": value}


def test_mix_accuracy_passing_by_bytes(tmp_path: Path) -> None:
    # The committed fixture holds 448 and 164 payload bytes of 612, configured at 75 and 25.
    config = tmp_path / "test.yaml"
    old = (mix_tests.FIXTURE / "test.yaml").read_text()
    config.write_text(old.replace("basis: sessions", "basis: bytes"))
    entry = mix_tests._fixture_entry("run-report.json", test_config=config)
    _assert_case(
        entry,
        True,
        [
            _mix("mix.share_pct", 73.202614, application="http1"),
            _mix("mix.share_pct", 26.797386, application="dns"),
            _mix("mix.abs_error_pp", 1.797386, application="http1"),
            _mix("mix.abs_error_pp", 1.797386, application="dns"),
            _mix("mix.max_abs_error_pp", 1.797386),
            _failures("mix_accuracy", 0),
        ],
    )


def test_mix_accuracy_failing_by_sessions() -> None:
    # The capture holds 8 and 2 of 10 sessions against 75 and 25 percent. Each application
    # misses on both sources, and the report names four failure sentences.
    entry = mix_tests._fixture_entry("run-report.json")
    assert entry["failures_total"] == 4
    _assert_case(
        entry,
        False,
        [
            _mix("mix.share_pct", 80.0, application="http1"),
            _mix("mix.share_pct", 20.0, application="dns"),
            _mix("mix.abs_error_pp", 5.0, application="http1"),
            _mix("mix.abs_error_pp", 5.0, application="dns"),
            _mix("mix.max_abs_error_pp", 5.0),
            _failures("mix_accuracy", 4),
        ],
    )


def test_mix_accuracy_without_a_basis_carries_only_the_failures_series(tmp_path: Path) -> None:
    labels = tmp_path / "labels.json"
    weights = {"http1/web_get": 3, "dns/lookup": 1}
    labels.write_text(json.dumps(mix_tests._labels(basis="both", weights=weights)))
    report = score(
        mix_tests.FIXTURE / "mix.pcapng",
        labels,
        mix_tests.STUB,
        run_report=mix_tests.FIXTURE / "run-report.json",
    )
    entry = mix_tests._entry_of(report.to_dict())
    assert entry["metrics_basis"] is None
    _assert_case(entry, entry["passed"], [_failures("mix_accuracy", entry["failures_total"])])


def test_a_failed_check_never_reports_zero_failures() -> None:
    assert series.failures_series("mix_accuracy", 0, False)["value"] == 1
    assert series.failures_series("mix_accuracy", 7, False)["value"] == 7
    assert series.failures_series("mix_accuracy", 0, True)["value"] == 0


# --- transport_behavior ---------------------------------------------------------------------

LINUX_FIELDS = ("ttl", "df", "mss", "window_scale", "sack", "timestamps")


def _deviations(**counts: int) -> Series:
    """Every field of both roles with its count: ``ttl_client=5`` sets one of them."""
    out = []
    for side in ("client", "server"):
        for name in LINUX_FIELDS:
            labels = {"property": name, "side": side}
            value = counts.get(f"{name}_{side}", 0)
            out.append({"name": "realism.transport_deviations", "labels": labels, "value": value})
    return out


def test_transport_behavior_passing(tmp_path: Path) -> None:
    syn, syn_ack = transport_tests.STACKS["linux6"]
    cap = transport_tests._capture(syn, syn_ack)
    entry = transport_tests._run(tmp_path, cap, transport_tests._labels()).to_dict()
    _assert_case(entry, True, [*_deviations(), _failures("transport_behavior", 0)])


def test_transport_behavior_failing_counts_the_packets_of_the_role(tmp_path: Path) -> None:
    # Five SYN packets carry TTL 64 where the Windows 11 profile expects 128: one deviation
    # entry, so one failure sentence, and five packets in the series.
    syn, syn_ack = transport_tests.STACKS["windows11"]
    cap = transport_tests._capture(pcapgen.Handshake(64, syn.window, syn.options), syn_ack)
    labels = transport_tests._profile_labels("windows11")
    entry = transport_tests._run(tmp_path, cap, labels).to_dict()
    _assert_case(entry, False, [*_deviations(ttl_client=5), _failures("transport_behavior", 1)])


def test_transport_behavior_series_cover_strict_fields_only_in_strict_mode(
    tmp_path: Path,
) -> None:
    syn, syn_ack = transport_tests.STACKS["linux6"]
    cap = transport_tests._capture(syn, syn_ack)
    relaxed = transport_tests._run(tmp_path, cap, transport_tests._labels()).to_dict()
    strict = transport_tests._run(tmp_path, cap, transport_tests._labels(), strict=True).to_dict()
    names = {m["labels"].get("property") for m in relaxed["metrics"]}
    assert not names & {"options", "syn_window"}
    extra = [
        m for m in strict["metrics"] if m["labels"].get("property") in ("options", "syn_window")
    ]
    assert [(m["labels"]["property"], m["labels"]["side"], m["value"]) for m in extra] == [
        ("options", "client", 0),
        ("syn_window", "client", 0),
    ]
    _validate_entry(strict)


def test_transport_behavior_failing_without_a_handshake_carries_only_the_failures_series(
    tmp_path: Path,
) -> None:
    entry = transport_tests._run(tmp_path, pcapgen.Capture(), transport_tests._labels()).to_dict()
    assert entry["syns_total"] == 0
    _assert_case(entry, False, [_failures("transport_behavior", 1)])


# --- the schema -----------------------------------------------------------------------------


def test_the_report_format_is_0_14_0() -> None:
    assert REPORT_VERSION == "0.15.0"


def test_a_check_without_its_failures_series_is_refused(tmp_path: Path) -> None:
    entry = _conformance(tmp_path, "DNS")
    _validate_entry(entry)
    changes: list[Callable[[dict[str, Any]], object]] = [
        lambda e: e["metrics"].pop(),  # no failures series
        lambda e: e["metrics"].append(_failures("conformance", 0)),  # two of them
        lambda e: e.update(passed=False),  # value 0 for a failed check
        lambda e: e["metrics"][-1].update(value=1),  # value 1 for a passed check
        lambda e: e["metrics"][-1]["labels"].update(check="temporal"),  # another kind
        lambda e: e["metrics"][-1].update(value=-1),
    ]
    for change in changes:
        broken = json.loads(json.dumps(entry))
        change(broken)
        with pytest.raises(jsonschema.ValidationError):
            _validate_entry(broken)


@pytest.mark.parametrize(
    "check",
    [
        "parser_errors",
        "encryption",
        "content",
        "session_structure",
        "temporal",
        "mix_accuracy",
        "transport_behavior",
    ],
)
def test_every_kind_requires_its_failures_series(check: str) -> None:
    metrics = SCHEMA["$defs"][check]["properties"]["metrics"]
    assert "contains" in metrics, check
    assert "maxItems" not in metrics, check
    contains = json.dumps(metrics["contains"])
    assert "realism.check_failures" in contains and f'"const": "{check}"' in contains


def test_the_catalog_names_every_check_kind_that_carries_a_failures_series() -> None:
    catalog = yaml.safe_load((REPO / "contracts" / "metrics" / "catalog.yaml").read_text())
    values = catalog["labels"]["check"]["values"]
    assert "messages" in values
    assert "host_identity" not in values  # a separate future package adds it
    assert set(series.CATALOG_CHECKS) == set(values)
    assert series.failures_series("messages", 2, False)["value"] == 2
    with pytest.raises(ValueError, match="no check kind"):
        series.failures_series("host_identity", 1, False)


def _messages_entry(tmp_path: Path, missing: int) -> dict[str, Any]:
    view, sessions = messages_tests._chunk_view(tmp_path, [587, 587, 587])
    counts = messages_tests._counts(sessions, p587={"messages": 1})
    for session in sessions[:missing]:
        del counts[session.key]
    result = messages.check(
        view, sessions, parse(messages_tests._doc()), messages.StubMessageProbe(counts)
    )
    entry = result.to_dict()
    assert entry["failures_total"] == missing
    return entry


def test_messages_passing_carries_the_failures_series_with_value_0(tmp_path: Path) -> None:
    _assert_case(_messages_entry(tmp_path, 0), True, [_failures("messages", 0)])


def test_messages_failing_carries_the_failures_series_equal_to_failures_total(
    tmp_path: Path,
) -> None:
    _assert_case(_messages_entry(tmp_path, 2), False, [_failures("messages", 2)])


def test_the_messages_check_requires_exactly_one_failures_series(tmp_path: Path) -> None:
    entry = _messages_entry(tmp_path, 1)
    _validate_entry(entry)
    changes: list[Callable[[dict[str, Any]], object]] = [
        lambda e: e["metrics"].pop(),  # none
        lambda e: e["metrics"].append(_failures("messages", 1)),  # two
        lambda e: e["metrics"].__setitem__(0, _failures("messages", 0)),  # 0 for a failed check
        lambda e: e["metrics"].__setitem__(0, _failures("temporal", 1)),  # another kind
    ]
    for change in changes:
        broken = json.loads(json.dumps(entry))
        change(broken)
        with pytest.raises(jsonschema.ValidationError):
            _validate_entry(broken)


# --- the examples of CON-23 -----------------------------------------------------------------

EXAMPLES_DOC = yaml.safe_load(EXAMPLES.read_text())
CASES = [
    pytest.param(example["check"], case, id=f"{example['check']}-{case['name']}")
    for example in EXAMPLES_DOC["examples"]
    for case in example["cases"]
]
GENERIC = next(
    entry
    for entry in SCHEMA["$defs"]["check"]["allOf"]
    if "then" in entry and "enum" in json.dumps(entry["if"])
)


@pytest.mark.parametrize(("kind", "case"), CASES)
def test_the_series_of_the_contract_examples_validate_against_the_schema(
    kind: str, case: dict[str, Any]
) -> None:
    metrics = case["metrics"]
    resolver_schema = {"$defs": SCHEMA["$defs"]}
    own = {"$ref": f"#/$defs/{kind}/properties/metrics", **resolver_schema}
    jsonschema.Draft202012Validator(own).validate(metrics)
    wrapper = {"allOf": [GENERIC], **resolver_schema}
    entry = {"check": kind, "passed": case["check_passed"], "metrics": metrics}
    jsonschema.Draft202012Validator(wrapper).validate(entry)
    failures = [m for m in metrics if m["name"] == "realism.check_failures"]
    assert len(failures) == 1 and failures[0]["labels"] == {"check": kind}


def test_the_contract_examples_cover_every_kind_with_a_passing_and_a_failing_case() -> None:
    kinds = {
        e["check"]: {c["name"].split()[0] for c in e["cases"]} for e in EXAMPLES_DOC["examples"]
    }
    assert set(kinds) == set(series.CATALOG_CHECKS)
    assert all({"passing", "failing"} <= names for names in kinds.values())


def test_the_runner_series_equal_the_contract_examples_where_the_cases_coincide(
    tmp_path: Path,
) -> None:
    """The runner's passing parser_errors case and the contract's differ only by application."""
    examples = {e["check"]: e for e in EXAMPLES_DOC["examples"]}
    failing_without_parser = next(
        c for c in examples["parser_errors"]["cases"] if c["name"] == "failing without a parser"
    )
    entry = _parser_errors(tmp_path, [], none=True)
    assert entry["metrics"] == failing_without_parser["metrics"]
