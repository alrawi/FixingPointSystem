# SPDX-License-Identifier: GPL-3.0-only
"""Replay of generated scenarios by the real agent (acceptance test of CAP-02).

The test needs the agent binary, root, tcpdump, and for the ``netns`` mode the namespace
harness of QA-01 (``ip`` from iproute2). A plain ``make test-cap`` run has none of them, so
the test skips with a message that says what to set. CI runs it as root with
``FPS_CAP_REPLAY=netns FPS_AGENT=target/debug/fps-agent`` (see docs/wiki/Capture.md).
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path

import pytest

from .replay import replay

MODE = os.environ.get("FPS_CAP_REPLAY", "")
AGENT = os.environ.get("FPS_AGENT", "")


def _skip_reason() -> str | None:
    how = (
        "set FPS_CAP_REPLAY=netns (root, ip, tcpdump) or loopback (root, tcpdump) and "
        "FPS_AGENT=path/to/fps-agent; CI does, see docs/wiki/Capture.md"
    )
    if MODE not in ("netns", "loopback"):
        return f"replay test not requested: {how}"
    if not AGENT or not Path(AGENT).is_file():
        return f"FPS_AGENT does not name the agent binary: {how}"
    if os.geteuid() != 0:
        return "the replay needs root for tcpdump and namespaces: run it with sudo"
    if shutil.which("tcpdump") is None:
        return "the replay needs tcpdump"
    if MODE == "netns" and shutil.which("ip") is None:
        return "the namespace harness needs `ip` from iproute2; use FPS_CAP_REPLAY=loopback here"
    return None


@pytest.mark.skipif(_skip_reason() is not None, reason=_skip_reason() or "")
def test_generated_scenarios_replay_with_the_captured_byte_counts(tmp_path: Path) -> None:
    problems = replay(Path(AGENT).resolve(), MODE, tmp_path)
    assert problems == []
