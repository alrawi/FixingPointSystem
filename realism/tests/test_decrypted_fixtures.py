# SPDX-License-Identifier: GPL-3.0-only
"""The decrypted command on the committed fixtures of the modules, with the real tools.

Each module with TLS sessions has a fixture capture that embeds its TLS keys, and
``realism/examples/decrypted/`` holds the labels of its decrypted view. These tests run
``python3 -m realism decrypted`` on the fixtures with the installed tshark and the
ndpiReader of nDPI 4.14, which replaces the ``decrypted_check.py`` copies of the modules. The
labels also give the message counts per session, which ``test_messages.py`` checks with tshark
alone.
They skip where a tool is missing or nDPI is older than 4.14, and the http2 fixture
comes from the same place as in ``test_ndpi_http2.py``.
"""

from __future__ import annotations

import json
import os
import re
import shutil
from pathlib import Path

import pytest

from realism.classifier import NdpiClassifier
from realism.cli import EXIT_PASS, main
from realism.tests.test_ndpi_http2 import FIXTURES, ROOT, _fixture

LABELS = ROOT / "realism" / "examples" / "decrypted"
READER = os.environ.get("FPS_NDPI_READER") or shutil.which("ndpiReader")
TSHARK = os.environ.get("FPS_TSHARK") or shutil.which("tshark")

# Module, sessions in the TLS part of the fixture, and the options of its view.
CASES = [
    ("http1", 80, []),
    ("smtp", 8, ["--plaintext", "upgrade", "--view-port", "587"]),
    ("imap", 8, ["--view-port", "143"]),
    ("http2", 40, []),
]


def _ndpi_is_current() -> bool:
    if READER is None:
        return False
    found = re.match(r"(\d+)\.(\d+)", NdpiClassifier(reader=READER).version())
    return bool(found) and (int(found.group(1)), int(found.group(2))) >= (4, 14)  # type: ignore[union-attr]


@pytest.mark.skipif(TSHARK is None, reason="tshark is not installed")
@pytest.mark.skipif(not _ndpi_is_current(), reason="needs ndpiReader 4.14 or newer")
@pytest.mark.parametrize(("module", "sessions", "options"), CASES, ids=[c[0] for c in CASES])
def test_the_shared_decrypted_check_passes_on_the_fixture(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    module: str,
    sessions: int,
    options: list[str],
) -> None:
    tree = ROOT / FIXTURES.replace("http2", module)
    if module == "http2":
        tree = _fixture(tmp_path)
    capture = tree / f"{module}.pcapng"
    if not capture.is_file():
        pytest.skip(f"{capture} is not in this checkout")
    out = tmp_path / "result.json"
    code = main(
        [
            "decrypted",
            "--pcap",
            str(capture),
            "--labels",
            str(LABELS / f"{module}.labels.yaml"),
            "--sessions",
            str(sessions),
            "--parsers",
            "tshark",
            "--output",
            str(out),
            *options,
        ]
    )
    result = json.loads(out.read_text())
    with capsys.disabled():
        app = result["report"]["checks"][0]["applications"][0]
        print(
            f"\nrealism decrypted {module}: {result['decrypted_sessions']} of "
            f"{result['tls_sessions']} TLS sessions decrypt, {result['classifier']} names "
            f"{app['conforming']} of {app['sessions']} view sessions {app['accepted']}"
        )
    assert code == EXIT_PASS, result["failures"]
    assert result["tls_sessions"] == result["decrypted_sessions"] == sessions
    # The labels carry literal message counts, which every view session must show.
    counts = next(c for c in result["report"]["checks"] if c["check"] == "messages")
    assert counts["passed"], counts["failures"]
    assert counts["sessions_checked"] == sessions
    assert counts["sessions_failed"] == 0
