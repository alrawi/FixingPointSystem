# SPDX-License-Identifier: GPL-3.0-only
"""Optional tests that run the real tshark and Zeek on synthetic captures.

Each test skips when its program is not installed. FPS_TSHARK and FPS_ZEEK name
programs outside the search path. The rlm workflow installs both, so CI runs them.
"""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

from realism.classifier import StubClassifier
from realism.parsers import TsharkRunner, ZeekRunner
from realism.report import score
from realism.tests import pcapgen

TSHARK = TsharkRunner()
ZEEK = ZeekRunner()
CLASSIFIER = StubClassifier.by_server_port({80: "HTTP", 53: "DNS"}, name="ndpi")
RUNNERS = [
    pytest.param(TSHARK, marks=pytest.mark.skipif(not TSHARK.available(), reason="no tshark")),
    pytest.param(ZEEK, marks=pytest.mark.skipif(not ZEEK.available(), reason="no zeek")),
]


def _parser_entry(pcap: Path, labels: Path, runner: Any) -> dict[str, Any]:
    report = score(pcap, labels, CLASSIFIER, parsers=[runner]).to_dict()
    (entry,) = [c for c in report["checks"] if c["check"] == "parser_errors"]
    assert isinstance(entry, dict)
    return entry


@pytest.mark.parametrize("runner", RUNNERS, ids=["tshark", "zeek"])
def test_real_parser_reports_one_error_for_one_malformed_dns_message(
    tmp_path: Path, labels_file: Callable[..., Path], runner: Any
) -> None:
    cap = pcapgen.mixed_capture(http_sessions=2, dns_lookups=2)
    cap.dns_malformed_answer(("10.0.0.10", 50100), ("10.0.1.53", 53), 0x2222)
    entry = _parser_entry(cap.write_pcapng(tmp_path / "bad.pcapng"), labels_file(), runner)
    by_app = {a["application"]: a for a in entry["applications"]}
    assert entry["parsers"][0]["version"] != "unknown"
    assert by_app["dns"]["parser_errors"] == 1, entry["errors"]
    assert by_app["http1"]["parser_errors"] == 0
    assert entry["errors_total"] == 1
    assert entry["errors"][0]["session"] == 5
    if runner.name == "tshark":
        assert by_app["dns"]["malformed_packets"] == 1


@pytest.mark.parametrize("runner", RUNNERS, ids=["tshark", "zeek"])
@pytest.mark.parametrize("fmt", ["pcap", "pcapng"])
def test_real_parser_finds_no_error_in_clean_traffic(
    tmp_path: Path, labels_file: Callable[..., Path], runner: Any, fmt: str
) -> None:
    cap = pcapgen.mixed_capture(http_sessions=3, dns_lookups=3)
    cap.http_get(("2001:db8::10", 41000), ("2001:db8::80", 80))
    path = tmp_path / f"clean.{fmt}"
    pcap = cap.write_pcap(path) if fmt == "pcap" else cap.write_pcapng(path)
    entry = _parser_entry(pcap, labels_file(), runner)
    assert entry["errors_total"] == 0, entry["errors"]
    assert entry["passed"] is True
