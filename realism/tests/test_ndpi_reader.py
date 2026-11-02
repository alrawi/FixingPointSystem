# SPDX-License-Identifier: GPL-3.0-only
"""Optional test that runs the real ndpiReader on a synthetic capture.

The test skips when ndpiReader is not installed. Set FPS_NDPI_READER to use a
reader outside the search path. The rlm workflow installs nDPI so that CI runs it.
"""

from __future__ import annotations

import os
import shutil
from collections.abc import Callable
from pathlib import Path

import pytest

from realism.classifier import NdpiClassifier
from realism.report import score
from realism.tests import pcapgen

READER = os.environ.get("FPS_NDPI_READER") or shutil.which("ndpiReader")

pytestmark = pytest.mark.skipif(READER is None, reason="ndpiReader is not installed")


@pytest.mark.parametrize("fmt", ["pcap", "pcapng"])
def test_real_ndpi_classifies_synthetic_http_and_dns(
    tmp_path: Path, labels_file: Callable[..., Path], fmt: str
) -> None:
    cap = pcapgen.mixed_capture(http_sessions=3, dns_lookups=3)
    cap.http_get(("2001:db8::10", 41000), ("2001:db8::80", 80))
    path = tmp_path / f"mix.{fmt}"
    pcap = cap.write_pcap(path) if fmt == "pcap" else cap.write_pcapng(path)
    report = score(pcap, labels_file(), NdpiClassifier(reader=READER)).to_dict()
    entry = report["checks"][0]
    assert entry["classifier_version"] != "Unknown"
    by_app = {a["application"]: a for a in entry["applications"]}
    assert by_app["http1"]["conforming"] == by_app["http1"]["sessions"] == 4
    assert by_app["dns"]["conforming"] == by_app["dns"]["sessions"] == 3
    assert report["passed"] is True


def test_real_ndpi_does_not_guess_from_the_port(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    cap = pcapgen.Capture()
    cap.tcp_session(("10.0.0.10", 40000), ("10.0.1.80", 80), b"\x00\x01binary\xff", b"\xfe\x00")
    report = score(
        cap.write_pcap(tmp_path / "opaque.pcap"), labels_file(), NdpiClassifier(reader=READER)
    )
    assert not report.passed
