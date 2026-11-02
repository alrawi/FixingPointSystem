# SPDX-License-Identifier: GPL-3.0-only
"""Shared fixtures of the realism tests."""

from __future__ import annotations

import json
import sys
from collections.abc import Callable
from pathlib import Path

import pytest
from hypothesis import settings
from hypothesis.configuration import set_hypothesis_home_dir

# Generated examples stay in memory, and Hypothesis keeps its other files in the
# ignored realism/.hypothesis directory, so test runs leave the tree clean.
set_hypothesis_home_dir(str(Path(__file__).resolve().parents[1] / ".hypothesis"))
settings.register_profile("fps", database=None)
settings.load_profile("fps")

NDPI_42_HEADER = (
    "#flow_id,protocol,first_seen,last_seen,duration,src_ip,src_port,dst_ip,dst_port,"
    "ndpi_proto_num,ndpi_proto,server_name_sni,c_to_s_pkts,c_to_s_bytes"
)

LABELS_HTTP_DNS = {
    "labels": "fps-realism-labels",
    "version": 1,
    "sessions": [
        {"application": "http1", "match": {"transport": "tcp", "server_port": 80}},
        {"application": "dns", "match": {"transport": "udp", "server_port": 53}},
    ],
}


@pytest.fixture
def labels_file(tmp_path: Path) -> Callable[..., Path]:
    """Return a factory that writes a labels document as JSON."""

    def write(doc: dict[str, object] | None = None, name: str = "labels.json") -> Path:
        path = tmp_path / name
        path.write_text(json.dumps(doc if doc is not None else LABELS_HTTP_DNS))
        return path

    return write


@pytest.fixture
def fake_reader(tmp_path: Path) -> Callable[..., Path]:
    """Return a factory for an executable that imitates ``ndpiReader``.

    The fake answers ``-h`` with a version banner and writes the given CSV text to
    the path after ``-C``. It can also exit with an error or sleep, so tests reach
    every branch of the process boundary without nDPI.
    """

    def make(csv_text: str = NDPI_42_HEADER + "\n", exit_code: int = 0, sleep_s: float = 0) -> Path:
        script = tmp_path / "fake-ndpiReader"
        csv_file = tmp_path / "fake-flows.csv"
        csv_file.write_text(csv_text)
        script.write_text(
            f"#!{sys.executable}\n"
            "import shutil, sys, time\n"
            "args = sys.argv[1:]\n"
            "if args == ['-h']:\n"
            "    print('Welcome to nDPI 9.8.7-fake')\n"
            "    sys.exit(0)\n"
            f"time.sleep({sleep_s})\n"
            f"if {exit_code}:\n"
            "    print('pcap open failed', file=sys.stderr)\n"
            f"    sys.exit({exit_code})\n"
            "assert '-d' in args, 'the runner must disable protocol guessing'\n"
            f"shutil.copy({str(csv_file)!r}, args[args.index('-C') + 1])\n"
        )
        script.chmod(0o755)
        return script

    return make
