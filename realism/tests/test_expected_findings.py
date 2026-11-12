# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the expected parser findings of the labels (RLM-20, issue #491).

A module whose traffic is malformed on purpose declares, per session rule, the Zeek weirds and
the tshark expert items that its sessions must produce. A declared finding does not count, a
declared finding that does not show fails the check, and an undeclared finding fails it too.
"""

from __future__ import annotations

import copy
import shutil
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from realism.capture import summarize
from realism.classifier import StubClassifier
from realism.labels import LabelsError, parse
from realism.parser_errors import ParserErrorsResult, check
from realism.parsers import KIND_WEIRD, Finding, StubParser, TsharkRunner
from realism.report import SCHEMA_PATH, score
from realism.tests import pcapgen
from realism.tests.conftest import LABELS_HTTP_DNS
from realism.tests.test_parser_errors import (
    DNS_CLIENT,
    DNS_SERVER,
    _malformed_dns_capture,
    _parser_entry,
    _reply_finding,
    _validate,
)

CLASSIFIER = StubClassifier.by_server_port({80: "HTTP", 53: "DNS"}, name="ndpi")
LABELS_SCHEMA = SCHEMA_PATH.parent / "labels.schema.json"


def _labels(expect: dict[str, list[str]] | None, rule: int = 1) -> Any:
    doc = copy.deepcopy(LABELS_HTTP_DNS)
    if expect is not None:
        doc["sessions"][rule]["expect_findings"] = expect  # type: ignore[index]
    return parse(doc)


def _run(pcap: Path, labels: Any, *parsers: StubParser) -> ParserErrorsResult:
    return check(pcap, summarize(pcap).sessions, labels, list(parsers))


def _weird(name: str, **over: Any) -> Finding:
    fields: dict[str, Any] = dict(
        parser="zeek",
        kind=KIND_WEIRD,
        name=name,
        timestamp=1.0,
        transport="udp",
        src_ip=DNS_CLIENT[0],
        src_port=DNS_CLIENT[1],
        dst_ip=DNS_SERVER[0],
        dst_port=DNS_SERVER[1],
    )
    fields.update(over)
    return Finding(**fields)


def _tshark(*findings: Finding) -> StubParser:
    return StubParser(list(findings), name="tshark")


def _zeek(*findings: Finding) -> StubParser:
    return StubParser(list(findings), name="zeek")


def test_a_declared_finding_is_reported_and_does_not_count(tmp_path: Path) -> None:
    pcap = _malformed_dns_capture(tmp_path)
    result = _run(pcap, _labels({"tshark": ["dns"]}), _tshark(_reply_finding("tshark")))
    assert result.passed
    entry = result.to_dict()
    assert (entry["errors_total"], entry["errors"]) == (0, [])
    assert entry["expected_total"] == 1
    (found,) = entry["expected_findings"]
    assert (found["parser"], found["application"], found["session_rule"]) == ("tshark", "dns", 1)
    assert entry["missing_findings"] == [] and entry["unchecked_expectations"] == []
    by_app = {a["application"]: a for a in entry["applications"]}
    assert by_app["dns"]["parser_errors"] == by_app["dns"]["malformed_packets"] == 0
    series = {(m["name"], m["labels"].get("application")): m["value"] for m in entry["metrics"]}
    assert series[("realism.parser_errors", "all")] == 0
    assert series[("realism.check_failures", None)] == 0


def test_a_declared_finding_that_does_not_show_fails(tmp_path: Path) -> None:
    result = _run(_malformed_dns_capture(tmp_path), _labels({"tshark": ["dns"]}), _tshark())
    assert not result.passed
    entry = result.to_dict()
    assert entry["errors_total"] == 0
    assert entry["missing_findings"] == [
        {"session_rule": 1, "application": "dns", "parser": "tshark", "item": "dns"}
    ]
    series = {(m["name"], m["labels"].get("application")): m["value"] for m in entry["metrics"]}
    assert series[("realism.check_failures", None)] == 1


def test_an_undeclared_finding_fails_next_to_a_declared_one(tmp_path: Path) -> None:
    pcap = _malformed_dns_capture(tmp_path)
    other = _reply_finding("tshark", name="udp", message="Bad length", frame=27)
    result = _run(pcap, _labels({"tshark": ["dns"]}), _tshark(_reply_finding("tshark"), other))
    assert not result.passed
    entry = result.to_dict()
    assert (entry["errors_total"], entry["expected_total"]) == (1, 1)
    assert entry["errors"][0]["name"] == "udp"
    assert entry["missing_findings"] == []


def test_without_a_declaration_the_finding_counts_as_before(tmp_path: Path) -> None:
    result = _run(
        _malformed_dns_capture(tmp_path), _labels(None), _tshark(_reply_finding("tshark"))
    )
    entry = result.to_dict()
    assert not result.passed and entry["errors_total"] == 1 and entry["expected_total"] == 0


def test_a_declaration_of_another_rule_does_not_cover_the_session(tmp_path: Path) -> None:
    labels = _labels({"tshark": ["dns"]}, rule=0)
    result = _run(_malformed_dns_capture(tmp_path), labels, _tshark(_reply_finding("tshark")))
    entry = result.to_dict()
    assert (entry["errors_total"], entry["expected_total"]) == (1, 0)
    assert entry["missing_findings"][0]["session_rule"] == 0


def test_a_tshark_item_names_the_protocol_and_a_part_of_the_message(tmp_path: Path) -> None:
    pcap = _malformed_dns_capture(tmp_path)
    finding = _reply_finding("tshark", message="Malformed Packet (Exception occurred)")
    hit = _run(pcap, _labels({"tshark": ["dns: Exception occurred"]}), _tshark(finding))
    assert hit.passed and hit.expected_total == 1
    miss = _run(pcap, _labels({"tshark": ["dns: Bad length"]}), _tshark(finding))
    assert not miss.passed
    assert (miss.errors_total, len(miss.missing_findings)) == (1, 1)
    wrong = _run(pcap, _labels({"tshark": ["http"]}), _tshark(finding))
    assert (wrong.errors_total, len(wrong.missing_findings)) == (1, 1)


def test_a_zeek_finding_is_covered_only_when_every_weird_is_declared(tmp_path: Path) -> None:
    pcap = _malformed_dns_capture(tmp_path)
    both = _labels({"zeek": ["DNS_RR_length_mismatch", "bad_UDP_checksum"]})
    covered = _run(pcap, both, _zeek(_weird("DNS_RR_length_mismatch,bad_UDP_checksum")))
    assert covered.passed and covered.expected_total == 1
    partial = _labels({"zeek": ["DNS_RR_length_mismatch"]})
    part = _run(pcap, partial, _zeek(_weird("DNS_RR_length_mismatch,bad_UDP_checksum")))
    # The weird that is declared did show, so only the undeclared one fails.
    assert not part.passed
    assert (part.errors_total, part.expected_total, part.missing_findings) == (1, 0, [])


def test_an_item_is_checked_per_parser(tmp_path: Path) -> None:
    pcap = _malformed_dns_capture(tmp_path)
    labels = _labels({"tshark": ["dns"], "zeek": ["DNS_RR_length_mismatch"]})
    result = _run(pcap, labels, _tshark(_reply_finding("tshark")), _zeek())
    assert [m["parser"] for m in result.to_dict()["missing_findings"]] == ["zeek"]
    assert not result.passed


def test_an_item_of_a_parser_that_did_not_run_is_unchecked_and_does_not_fail(
    tmp_path: Path,
) -> None:
    labels = _labels({"tshark": ["dns"], "zeek": ["DNS_RR_length_mismatch"]})
    result = _run(_malformed_dns_capture(tmp_path), labels, _tshark(_reply_finding("tshark")))
    assert result.passed
    entry = result.to_dict()
    assert [u["parser"] for u in entry["unchecked_expectations"]] == ["zeek"]
    assert entry["missing_findings"] == []


def test_a_declared_item_without_any_parser_still_fails_for_the_missing_parser(
    tmp_path: Path,
) -> None:
    result = _run(_malformed_dns_capture(tmp_path), _labels({"tshark": ["dns"]}))
    assert not result.passed and result.failures_total == 1
    assert len(result.unchecked) == 1


def test_a_declared_finding_wins_over_the_port_collision_rule(tmp_path: Path) -> None:
    pcap = _malformed_dns_capture(tmp_path)
    parser = StubParser(
        [_reply_finding("tshark")], name="tshark", bindings={("udp", DNS_CLIENT[1]): "manolito"}
    )
    result = _run(pcap, _labels({"tshark": ["dns"]}), parser)
    assert result.passed
    assert (result.expected_total, result.not_counted_total) == (1, 0)


def test_the_expected_list_is_capped_but_the_total_counts_all(tmp_path: Path) -> None:
    findings = [_reply_finding("tshark", frame=n) for n in range(1, 106)]
    result = _run(
        _malformed_dns_capture(tmp_path), _labels({"tshark": ["dns"]}), _tshark(*findings)
    )
    entry = result.to_dict()
    assert result.passed
    assert (len(entry["expected_findings"]), entry["expected_total"]) == (100, 105)


def test_the_report_validates_and_closes_its_new_entries(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    pcap = _malformed_dns_capture(tmp_path)
    doc = copy.deepcopy(LABELS_HTTP_DNS)
    doc["sessions"][1]["expect_findings"] = {  # type: ignore[index]
        "tshark": ["dns"],
        "zeek": ["bad_UDP_checksum"],
    }
    report = score(
        pcap, labels_file(doc), CLASSIFIER, parsers=[_tshark(_reply_finding("tshark")), _zeek()]
    ).to_dict()
    _validate(report)
    entry = _parser_entry(report)
    assert report["passed"] is False
    assert len(entry["missing_findings"]) == 1 and entry["expected_total"] == 1
    entry["missing_findings"][0]["surprise"] = 1
    with pytest.raises(jsonschema.ValidationError):
        _validate(report)


@pytest.mark.parametrize(
    ("expect", "message"),
    [
        ("dns", "must map a parser"),
        ({"stub": ["dns"]}, "names the parser"),
        ({"zeek": []}, "must list an item"),
        ({"zeek": ["bad request"]}, "no weird name"),
        ({"zeek": ["a", "a"]}, "twice"),
        ({"tshark": ["http:no space"]}, "must be a protocol name"),
        ({"tshark": [""]}, "non empty text"),
        ({"tshark": "http"}, "must be a list"),
    ],
)
def test_the_labels_refuse_a_malformed_declaration(expect: Any, message: str) -> None:
    with pytest.raises(LabelsError, match=message):
        _labels(expect)


def test_the_labels_schema_matches_the_parser_of_the_labels() -> None:
    import json

    schema = json.loads(LABELS_SCHEMA.read_text())
    validator = jsonschema.Draft202012Validator(schema)
    doc = copy.deepcopy(LABELS_HTTP_DNS)
    doc["sessions"][1]["expect_findings"] = {  # type: ignore[index]
        "zeek": ["bad_HTTP_request"],
        "tshark": ["http", "http: Illegal characters"],
    }
    validator.validate(doc)
    parse(doc)
    for bad in ({"zeek": ["bad request"]}, {"tshark": ["http:x"]}, {}, {"stub": ["x"]}):
        doc["sessions"][1]["expect_findings"] = bad  # type: ignore[index]
        with pytest.raises(jsonschema.ValidationError):
            validator.validate(doc)


@pytest.mark.skipif(shutil.which("tshark") is None, reason="tshark is not installed")
def test_real_tshark_declared_malformed_traffic_passes_and_the_rest_fails(tmp_path: Path) -> None:
    """Acceptance of RLM-20 with the real tshark on a deliberately malformed DNS answer."""
    pcap = _malformed_dns_capture(tmp_path)

    def run(expect: dict[str, list[str]] | None, capture: Path = pcap) -> ParserErrorsResult:
        return _run(capture, _labels(expect), _tshark_runner())

    declared = run({"tshark": ["dns"]})
    assert declared.passed and declared.errors_total == 0 and declared.expected_total == 1
    assert declared.to_dict()["expected_findings"][0]["application"] == "dns"

    undeclared = run(None)
    assert not undeclared.passed and undeclared.errors_total == 1

    wrong_item = run({"tshark": ["http"]})
    assert not wrong_item.passed
    assert (wrong_item.errors_total, len(wrong_item.missing_findings)) == (1, 1)

    clean = pcapgen.mixed_capture(http_sessions=2, dns_lookups=2).write_pcap(tmp_path / "ok.pcap")
    missing = run({"tshark": ["dns"]}, clean)
    assert not missing.passed
    assert (missing.errors_total, len(missing.missing_findings)) == (0, 1)


def _tshark_runner() -> Any:
    return TsharkRunner()
