# SPDX-License-Identifier: GPL-3.0-only
"""Component tests of the command line with a fake ndpiReader."""

from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path

import pytest

from realism.cli import EXIT_ERROR, EXIT_FAIL, EXIT_PASS, main
from realism.tests import pcapgen
from realism.tests.conftest import NDPI_42_HEADER

CSV_PASS = NDPI_42_HEADER + "\n" + "0,6,1,1,0,10.0.0.10,40000,10.0.1.80,80,7,HTTP,,6,414\n"


def _capture(tmp_path: Path) -> Path:
    cap = pcapgen.Capture()
    cap.http_get(("10.0.0.10", 40000), ("10.0.1.80", 80))
    return cap.write_pcap(tmp_path / "one.pcap")


def test_score_writes_a_passing_report(
    tmp_path: Path, fake_reader: Callable[..., Path], labels_file: Callable[..., Path]
) -> None:
    out = tmp_path / "report.json"
    argv = ["score", "--pcap", str(_capture(tmp_path)), "--labels", str(labels_file())]
    argv += ["--parsers", "none"]
    argv += ["--ndpi-reader", str(fake_reader(CSV_PASS)), "--output", str(out)]
    assert main(argv) == EXIT_PASS
    report = json.loads(out.read_text())
    assert report["passed"] is True
    assert report["checks"][0]["classifier_version"] == "9.8.7-fake"


def test_score_exits_1_when_conformance_fails(
    tmp_path: Path,
    fake_reader: Callable[..., Path],
    labels_file: Callable[..., Path],
    capsys: pytest.CaptureFixture[str],
) -> None:
    argv = ["score", "--pcap", str(_capture(tmp_path)), "--labels", str(labels_file())]
    argv += ["--parsers", "none"]
    argv += ["--ndpi-reader", str(fake_reader())]
    assert main(argv) == EXIT_FAIL
    report = json.loads(capsys.readouterr().out)
    assert report["checks"][0]["applications"][0]["observed"] == {"Unknown": 1}


def test_score_exits_2_on_input_errors(
    tmp_path: Path,
    fake_reader: Callable[..., Path],
    labels_file: Callable[..., Path],
    capsys: pytest.CaptureFixture[str],
) -> None:
    argv = ["score", "--pcap", str(tmp_path / "absent.pcap"), "--labels", str(labels_file())]
    assert main([*argv, "--ndpi-reader", str(fake_reader())]) == EXIT_ERROR
    assert "read capture" in capsys.readouterr().err
    argv = ["score", "--pcap", str(_capture(tmp_path)), "--labels", str(labels_file())]
    argv += ["--parsers", "none"]
    assert main([*argv, "--ndpi-reader", str(tmp_path / "none")]) == EXIT_ERROR
    assert "not found" in capsys.readouterr().err


def test_score_rejects_a_threshold_outside_0_to_1(tmp_path: Path) -> None:
    with pytest.raises(SystemExit) as exc:
        main(["score", "--pcap", "a", "--labels", "b", "--threshold", "1.5"])
    assert exc.value.code == 2
