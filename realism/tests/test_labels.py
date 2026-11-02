# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of the labels loader."""

from __future__ import annotations

import copy
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

from realism.capture import Session
from realism.labels import LabelsError, load, parse
from realism.tests.conftest import LABELS_HTTP_DNS


def _session(transport: str, server_port: int, server_ip: str = "10.0.1.1") -> Session:
    return Session(1, transport, "10.0.0.1", 40000, server_ip, server_port, 0.0, 0.0)


def test_first_matching_rule_names_the_application() -> None:
    labels = parse(LABELS_HTTP_DNS)
    assert labels.application_of(_session("tcp", 80)) == "http1"
    assert labels.application_of(_session("udp", 53)) == "dns"
    assert labels.application_of(_session("tcp", 53)) is None


def test_default_labels_unmatched_sessions() -> None:
    doc = dict(LABELS_HTTP_DNS, default="http1")
    assert parse(doc).application_of(_session("tcp", 8080)) == "http1"


def test_ip_match_is_normalized() -> None:
    doc = {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [{"application": "http1", "match": {"server_ip": "2001:DB8:0::80"}}],
    }
    assert parse(doc).application_of(_session("tcp", 80, "2001:db8::80")) == "http1"


def test_applications_override_the_builtin_names() -> None:
    doc = dict(LABELS_HTTP_DNS, applications={"http1": {"ndpi": ["HTTP", "HTTP_Proxy"]}})
    labels = parse(doc)
    assert labels.accepted_names("http1", "ndpi") == ("HTTP", "HTTP_Proxy")
    assert labels.accepted_names("dns", "ndpi") == ("DNS",)
    assert labels.accepted_names("smtp", "ndpi") == ()


def test_yaml_and_json_load_the_same_labels(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    yaml_path = tmp_path / "expected.yaml"
    yaml_path.write_text(
        "labels: fps-realism-labels\nversion: 1\nthreshold: 0.95\nsessions:\n"
        "  - application: http1\n    match: {transport: tcp, server_port: 80}\n"
        "  - application: dns\n    match: {transport: udp, server_port: 53}\n"
    )
    from_yaml = load(yaml_path)
    from_json = load(labels_file())
    assert from_yaml.rules == from_json.rules
    assert from_yaml.threshold == 0.95


def _broken(path: str, value: Any) -> dict[str, Any]:
    doc = copy.deepcopy(LABELS_HTTP_DNS)
    target: Any = doc
    keys = path.split(".")
    for key in keys[:-1]:
        target = target[int(key)] if key.isdigit() else target[key]
    last = keys[-1]
    if value is KeyError:
        del target[last]
    else:
        target[int(last) if last.isdigit() else last] = value
    return doc


@pytest.mark.parametrize(
    ("doc", "message"),
    [
        ([], "must be a mapping"),
        (_broken("labels", "other"), "field labels"),
        (_broken("version", 2), "field version"),
        (_broken("threshold", 1.5), "threshold"),
        (_broken("threshold", True), "threshold"),
        (_broken("extra", 1), "unknown fields"),
        (_broken("sessions", {}), "must be a list"),
        (_broken("sessions.0.application", ""), "application"),
        (_broken("sessions.0.match.server_port", 70000), "port number"),
        (_broken("sessions.0.match.server_port", "80"), "must be int"),
        (_broken("sessions.0.match.transport", "sctp"), "tcp or udp"),
        (_broken("sessions.0.match.vlan", 3), "unknown match field"),
        (_broken("sessions.0.match.server_ip", "not-an-ip"), "server_ip"),
        (_broken("applications", {"http1": {"ndpi": "HTTP"}}), "list of protocol names"),
    ],
)
def test_invalid_labels_are_rejected(doc: Any, message: str) -> None:
    with pytest.raises(LabelsError, match=message):
        parse(doc)


def test_unreadable_and_malformed_files(tmp_path: Path) -> None:
    with pytest.raises(LabelsError, match="read labels"):
        load(tmp_path / "absent.json")
    bad = tmp_path / "bad.json"
    bad.write_text("{")
    with pytest.raises(LabelsError, match="parse labels"):
        load(bad)
    bad_yaml = tmp_path / "bad.yaml"
    bad_yaml.write_text("labels: [")
    with pytest.raises(LabelsError, match="parse labels"):
        load(bad_yaml)


def test_example_labels_file_loads_and_validates() -> None:
    import json

    import jsonschema
    import yaml

    from realism.report import LABELS_SCHEMA_PATH

    example = Path(__file__).resolve().parents[1] / "examples" / "http-dns.labels.yaml"
    schema = json.loads(LABELS_SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator(schema).validate(yaml.safe_load(example.read_text()))
    assert load(example).rules == parse(LABELS_HTTP_DNS).rules
