# SPDX-License-Identifier: GPL-3.0-only
"""Component tests of the classifiers across the ndpiReader process boundary."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path

import pytest

from realism.capture import Session, SessionKey
from realism.classifier import (
    ClassifierError,
    NdpiClassifier,
    StubClassifier,
    matches,
    parse_ndpi_csv,
)
from realism.tests.conftest import NDPI_42_HEADER

ROWS = (
    "0,6,1.0,1.1,0.1,10.0.0.10,40000,10.0.1.80,80,7,HTTP,www.example.com,6,414\n"
    "1,17,1.0,1.1,0.1,10.0.0.10,50000,10.0.1.53,53,5,DNS.Google,,1,75\n"
    "2,6,1.0,1.1,0.1,2001:db8:0::1,41000,2001:db8::80,80,0,,,1,60\n"
    "3,1,1.0,1.1,0.1,10.0.0.10,0,10.0.1.1,0,81,ICMP,,1,60\n"
)


def test_parse_ndpi_42_csv() -> None:
    result = parse_ndpi_csv(NDPI_42_HEADER + "\n" + ROWS)
    assert result[SessionKey.of("tcp", "10.0.1.80", 80, "10.0.0.10", 40000)] == "HTTP"
    assert result[SessionKey.of("udp", "10.0.0.10", 50000, "10.0.1.53", 53)] == "DNS.Google"
    assert result[SessionKey.of("tcp", "2001:db8::1", 41000, "2001:db8::80", 80)] == "Unknown"
    assert len(result) == 3


def test_parse_finds_columns_by_name_in_later_layouts() -> None:
    header = "#flow_id,protocol,first_seen,last_seen,duration,src_ip,src_port,dst_ip,dst_port,"
    header += "ndpi_proto_num,ndpi_proto,proto_by_ip,proto_by_ip_num,server_name_sni"
    row = "0,6,1,1,0,10.0.0.1,1000,10.0.0.2,80,7,HTTP,Unknown,0,example.com"
    assert list(parse_ndpi_csv(header + "\n" + row).values()) == ["HTTP"]


@pytest.mark.parametrize(
    ("text", "message"),
    [
        ("#flow_id,protocol,src_ip\n", "header lacks"),
        (NDPI_42_HEADER + "\n0,6,1\n", "fields"),
        (NDPI_42_HEADER + "\n0,6,1,1,0,bad-ip,1,10.0.0.2,80,7,HTTP,,1,1\n", "line 2"),
    ],
)
def test_parse_rejects_malformed_csv(text: str, message: str) -> None:
    with pytest.raises(ClassifierError, match=message):
        parse_ndpi_csv(text)


def test_empty_csv_has_no_verdicts() -> None:
    assert parse_ndpi_csv("") == {}


@pytest.mark.parametrize(
    ("observed", "accepted", "expected"),
    [
        ("HTTP", ("HTTP",), True),
        ("http", ("HTTP",), True),
        ("DNS.Google", ("DNS",), True),
        ("TLS.Google", ("HTTP",), False),
        ("HTTP_Proxy", ("HTTP",), False),
        ("Unknown", (), False),
    ],
)
def test_matches(observed: str, accepted: tuple[str, ...], expected: bool) -> None:
    assert matches(observed, accepted) is expected


def test_stub_classifies_by_server_port() -> None:
    sessions = [
        Session(1, "tcp", "10.0.0.1", 40000, "10.0.0.2", 80, 0, 0),
        Session(2, "tcp", "10.0.0.1", 40001, "10.0.0.2", 443, 0, 0),
    ]
    stub = StubClassifier.by_server_port({80: "HTTP"}, name="ndpi")
    assert stub.name == "ndpi"
    assert stub.classify(Path("unused"), sessions) == {sessions[0].key: "HTTP"}


def test_ndpi_classifier_runs_the_reader_out_of_process(
    tmp_path: Path, fake_reader: Callable[..., Path]
) -> None:
    reader = fake_reader(NDPI_42_HEADER + "\n" + ROWS)
    classifier = NdpiClassifier(reader=str(reader))
    assert classifier.version() == "9.8.7-fake"
    result = classifier.classify(tmp_path / "any.pcap", [])
    assert len(result) == 3


def test_ndpi_reader_from_environment(
    monkeypatch: pytest.MonkeyPatch, fake_reader: Callable[..., Path]
) -> None:
    monkeypatch.setenv("FPS_NDPI_READER", str(fake_reader()))
    assert NdpiClassifier().version() == "9.8.7-fake"


def test_ndpi_reader_failure_is_reported(tmp_path: Path, fake_reader: Callable[..., Path]) -> None:
    classifier = NdpiClassifier(reader=str(fake_reader(exit_code=3)))
    with pytest.raises(ClassifierError, match="exited with 3: pcap open failed"):
        classifier.classify(tmp_path / "x.pcap", [])


def test_ndpi_reader_timeout_is_reported(tmp_path: Path, fake_reader: Callable[..., Path]) -> None:
    classifier = NdpiClassifier(reader=str(fake_reader(sleep_s=5)), timeout_s=0.5)
    with pytest.raises(ClassifierError, match="ran longer than"):
        classifier.classify(tmp_path / "x.pcap", [])


def test_missing_ndpi_reader_is_reported(tmp_path: Path) -> None:
    classifier = NdpiClassifier(reader=str(tmp_path / "no-such-reader"))
    with pytest.raises(ClassifierError, match="not found"):
        classifier.version()
