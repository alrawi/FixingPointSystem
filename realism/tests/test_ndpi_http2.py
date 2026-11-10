# SPDX-License-Identifier: GPL-3.0-only
"""Optional tests that the pinned nDPI 4.14 names HTTP/2 sessions HTTP2.

nDPI 4.2.0, the libndpi-bin of Ubuntu 24.04, has no HTTP/2 protocol and reports an
h2c session as Unknown. ``realism/ci/build-ndpi.sh`` builds nDPI 4.14 for CI, and
these tests run the real ndpiReader on HTTP/2 sessions with prior knowledge (h2c),
which carry the client preface in the clear. They skip where ndpiReader is missing
or older than 4.14, and they fail instead of skipping on a runner of GitHub Actions,
because the pin there must hold.

The test of the http2 module fixture reads ``modules/http2/fixtures/http2.pcapng``.
Until the http2 module is merged, a runner of GitHub Actions fetches the file from
the branch ``wp/MOD-06`` (``FPS_H2_FIXTURE_REF`` names another ref), and anywhere
else the test skips when the file is absent.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from pathlib import Path
from typing import Any

import pytest
import yaml

from realism.classifier import NdpiClassifier
from realism.report import score
from realism.tests import pcapgen

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = "modules/http2/fixtures"
READER = os.environ.get("FPS_NDPI_READER") or shutil.which("ndpiReader")
IN_CI = os.environ.get("GITHUB_ACTIONS") == "true"
PREFACE = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"
EMPTY_SETTINGS = b"\x00\x00\x00\x04\x00\x00\x00\x00\x00"
MIN_RATIO = 0.99


def _version() -> tuple[int, int] | None:
    if READER is None:
        return None
    found = re.match(r"(\d+)\.(\d+)", NdpiClassifier(reader=READER).version())
    return (int(found.group(1)), int(found.group(2))) if found else None


@pytest.fixture(scope="module")
def reader() -> str:
    version = _version()
    if version is None:
        if IN_CI:
            pytest.fail("ndpiReader is not installed on this runner")
        pytest.skip("ndpiReader is not installed")
    assert READER is not None
    if version < (4, 14):
        message = f"ndpiReader {version[0]}.{version[1]} has no HTTP2 protocol, 4.14 needs"
        if IN_CI:
            pytest.fail(message + " realism/ci/build-ndpi.sh")
        pytest.skip(message)
    return READER


def _announce(capsys: pytest.CaptureFixture[str], line: str) -> None:
    """Print a line that the log of a passing test run keeps, and add it to the job summary."""
    with capsys.disabled():
        print(f"\n{line}")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as out:
            out.write(f"{line}\n\n")


def _labels(tmp_path: Path, doc: dict[str, Any]) -> Path:
    path = tmp_path / "labels.json"
    path.write_text(json.dumps(doc))
    return path


def test_ndpi_414_names_synthetic_h2c_sessions_http2(
    tmp_path: Path, reader: str, capsys: pytest.CaptureFixture[str]
) -> None:
    cap = pcapgen.Capture()
    for i in range(10):
        cap.tcp_session(
            ("10.0.0.10", 40000 + i),
            ("10.0.1.80", 8082),
            PREFACE + EMPTY_SETTINGS,
            EMPTY_SETTINGS,
        )
    labels = _labels(
        tmp_path,
        {
            "labels": "fps-realism-labels",
            "version": 1,
            "sessions": [
                {"application": "http2", "match": {"transport": "tcp", "server_port": 8082}}
            ],
            "applications": {"http2": {"ndpi": ["HTTP2"]}},
        },
    )
    report = score(cap.write_pcap(tmp_path / "h2c.pcap"), labels, NdpiClassifier(reader=reader))
    entry = report.to_dict()["checks"][0]
    app = entry["applications"][0]
    _announce(
        capsys,
        f"nDPI {entry['classifier_version']} classifies {app['conforming']} of {app['sessions']} "
        f"synthetic h2c sessions as HTTP2",
    )
    assert app["sessions"] == app["conforming"] == 10, app["observed"]
    assert report.passed


def _fixture(tmp_path: Path) -> Path:
    """Return the directory that holds http2.pcapng and expected.yaml, or skip."""
    tree = ROOT / FIXTURES
    if (tree / "http2.pcapng").is_file():
        return tree
    ref = os.environ.get("FPS_H2_FIXTURE_REF") or ("wp/MOD-06" if IN_CI else "")
    if not ref:
        pytest.skip("modules/http2/fixtures/http2.pcapng is not in this checkout")
    fetched = subprocess.run(
        ["git", "-C", str(ROOT), "fetch", "--depth", "1", "origin", ref],
        capture_output=True,
        text=True,
        check=False,
    )
    if fetched.returncode != 0:
        pytest.skip(f"cannot fetch {ref}: {fetched.stderr.strip()[-200:]}")
    for name in ("http2.pcapng", "expected.yaml"):
        shown = subprocess.run(
            ["git", "-C", str(ROOT), "show", f"FETCH_HEAD:{FIXTURES}/{name}"],
            capture_output=True,
            check=False,
        )
        if shown.returncode != 0:
            pytest.skip(f"{ref} has no {FIXTURES}/{name}")
        (tmp_path / name).write_bytes(shown.stdout)
    return tmp_path


def test_ndpi_414_names_the_cleartext_sessions_of_the_http2_fixture_http2(
    tmp_path: Path, reader: str, capsys: pytest.CaptureFixture[str]
) -> None:
    fixture = _fixture(tmp_path)
    doc = yaml.safe_load((fixture / "expected.yaml").read_text(encoding="utf-8"))
    clear = [rule for rule in doc["sessions"] if rule["application"] == "http2"]
    assert clear, "expected.yaml labels no cleartext HTTP/2 session"
    labels = _labels(
        tmp_path,
        {
            "labels": doc["labels"],
            "version": doc["version"],
            "threshold": MIN_RATIO,
            "sessions": clear,
            "applications": {"http2": doc["applications"]["http2"]},
        },
    )
    report = score(fixture / "http2.pcapng", labels, NdpiClassifier(reader=reader))
    entry = report.to_dict()["checks"][0]
    app = entry["applications"][0]
    ports = sorted(rule["match"]["server_port"] for rule in clear)
    _announce(
        capsys,
        f"nDPI {entry['classifier_version']} classifies {app['conforming']} of {app['sessions']} "
        f"cleartext sessions of the http2 fixture (server ports {ports}) as HTTP2, "
        f"ratio {app['ratio']:.4f}, observed {dict(app['observed'])}",
    )
    assert app["sessions"] > 0
    assert app["ratio"] >= MIN_RATIO, app["observed"]
