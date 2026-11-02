# SPDX-License-Identifier: GPL-3.0-only
"""Component tests of the conformance check and the report with a stub classifier."""

from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest
import yaml

from realism.capture import Session
from realism.classifier import StubClassifier
from realism.conformance import MAX_MISMATCHES, check
from realism.labels import parse
from realism.report import LABELS_SCHEMA_PATH, SCHEMA_PATH, score
from realism.tests import pcapgen
from realism.tests.conftest import LABELS_HTTP_DNS

REPO = Path(__file__).resolve().parents[2]
CATALOG = REPO / "contracts" / "metrics" / "catalog.yaml"


def _validate(report: dict[str, Any]) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator.check_schema(schema)
    jsonschema.Draft202012Validator(schema).validate(report)


def _sessions(count: int, server_port: int = 80) -> list[Session]:
    return [
        Session(i + 1, "tcp", "10.0.0.1", 10000 + i, "10.0.0.2", server_port, 0, 0)
        for i in range(count)
    ]


def _first_n_http(n: int) -> StubClassifier:
    return StubClassifier(lambda s: "HTTP" if s.id <= n else "TLS", name="ndpi")


@pytest.mark.parametrize(("conforming", "passed"), [(100, True), (99, True), (98, False)])
def test_threshold_is_99_percent_of_sessions(conforming: int, passed: bool) -> None:
    result = check(
        Path("x"), _sessions(100), parse(LABELS_HTTP_DNS), _first_n_http(conforming), 0.99
    )
    (app,) = result.applications
    assert (app.sessions, app.conforming) == (100, conforming)
    assert result.passed is passed
    assert result.mismatches_total == 100 - conforming


def test_unrecognized_sessions_count_as_unknown() -> None:
    stub = StubClassifier(lambda s: None, name="ndpi")
    result = check(Path("x"), _sessions(3), parse(LABELS_HTTP_DNS), stub, 0.99)
    entry = result.to_dict()
    assert entry["applications"][0]["observed"] == {"Unknown": 3}
    assert entry["metrics"][0]["value"] == 0.0
    assert not result.passed


def test_unlabelled_sessions_do_not_count() -> None:
    sessions = _sessions(2) + _sessions(1, server_port=9999)
    result = check(Path("x"), sessions, parse(LABELS_HTTP_DNS), _first_n_http(3), 0.99)
    assert result.unlabelled_sessions == 1
    assert result.applications[0].sessions == 2
    assert result.passed


def test_no_labelled_sessions_fails() -> None:
    result = check(Path("x"), _sessions(2, 9999), parse(LABELS_HTTP_DNS), _first_n_http(2), 0.99)
    assert result.applications == []
    assert not result.passed


def test_application_without_accepted_names_fails() -> None:
    doc = dict(LABELS_HTTP_DNS, default="gopher")
    result = check(Path("x"), _sessions(1, 70), parse(doc), _first_n_http(1), 0.99)
    assert result.applications[0].accepted == ()
    assert not result.passed


def test_mismatch_list_is_truncated() -> None:
    count = MAX_MISMATCHES + 5
    result = check(Path("x"), _sessions(count), parse(LABELS_HTTP_DNS), _first_n_http(0), 0.99)
    assert len(result.mismatches) == MAX_MISMATCHES
    assert result.mismatches_total == count
    assert result.mismatches[0]["server"] == {"ip": "10.0.0.2", "port": 80}


def test_report_of_mixed_capture_validates_against_the_schema(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    pcap = pcapgen.mixed_capture(http_sessions=3, dns_lookups=2).write_pcap(tmp_path / "m.pcap")
    stub = StubClassifier.by_server_port({80: "HTTP", 53: "DNS"}, name="ndpi")
    report = score(pcap, labels_file(), stub)
    doc = report.to_dict()
    _validate(doc)
    assert report.passed
    assert doc["capture"]["sessions"] == 5
    by_app = {a["application"]: a for a in doc["checks"][0]["applications"]}
    assert (by_app["http1"]["sessions"], by_app["dns"]["sessions"]) == (3, 2)


def test_failing_report_validates_and_fails(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    pcap = pcapgen.mixed_capture().write_pcapng(tmp_path / "m.pcapng")
    stub = StubClassifier.by_server_port({80: "HTTP"}, name="ndpi")
    report = score(pcap, labels_file(), stub)
    _validate(report.to_dict())
    assert not report.passed
    assert report.to_dict()["checks"][0]["mismatches_total"] == 3


def test_threshold_precedence(tmp_path: Path, labels_file: Callable[..., Path]) -> None:
    pcap = pcapgen.mixed_capture(http_sessions=4, dns_lookups=0).write_pcap(tmp_path / "h.pcap")
    stub = StubClassifier(lambda s: "HTTP" if s.client_port != 40003 else None, name="ndpi")
    labels = labels_file(dict(LABELS_HTTP_DNS, threshold=0.75))
    assert score(pcap, labels, stub).passed
    assert not score(pcap, labels, stub, threshold=0.8).passed
    assert not score(pcap, labels_file(), stub).passed


def test_report_names_only_catalog_metrics_and_labels(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    catalog = yaml.safe_load(CATALOG.read_text())
    metrics = {m["name"]: m for m in catalog["metrics"]}
    classifiers = set(catalog["labels"]["classifier"]["values"])
    pcap = pcapgen.mixed_capture().write_pcap(tmp_path / "m.pcap")
    stub = StubClassifier.by_server_port({80: "HTTP", 53: "DNS"}, name="ndpi")
    for entry in score(pcap, labels_file(), stub).to_dict()["checks"]:
        for metric in entry["metrics"]:
            assert metric["name"] in metrics
            assert set(metric["labels"]) == set(metrics[metric["name"]]["labels"])
            assert metric["labels"]["classifier"] in classifiers


def test_schema_classifier_enum_covers_the_catalog() -> None:
    catalog = yaml.safe_load(CATALOG.read_text())
    schema = json.loads(SCHEMA_PATH.read_text())
    enum = set(schema["$defs"]["conformance"]["properties"]["classifier"]["enum"])
    assert set(catalog["labels"]["classifier"]["values"]) <= enum


def test_labels_schema_accepts_what_the_loader_accepts() -> None:
    schema = json.loads(LABELS_SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator.check_schema(schema)
    doc = dict(
        LABELS_HTTP_DNS,
        threshold=0.99,
        default="http1",
        applications={"http1": {"ndpi": ["HTTP"]}},
    )
    jsonschema.Draft202012Validator(schema).validate(doc)
    parse(doc)
    with pytest.raises(jsonschema.ValidationError):
        jsonschema.Draft202012Validator(schema).validate(dict(doc, version=2))


def test_unknown_check_kind_needs_only_the_common_fields() -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    validator = jsonschema.Draft202012Validator(
        {**schema["$defs"]["check"], "$defs": schema["$defs"]}
    )
    validator.validate({"check": "parser_errors", "passed": True, "metrics": [], "extra": 1})
    with pytest.raises(jsonschema.ValidationError):
        validator.validate({"check": "conformance", "passed": True, "metrics": []})
