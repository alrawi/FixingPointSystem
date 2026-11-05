# SPDX-License-Identifier: GPL-3.0-only
"""The Zeek pin of the CI workflows lives in one file and nowhere else."""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PIN = ROOT / "realism" / "ci" / "zeek.version"
SCRIPT = ROOT / "realism" / "ci" / "install-tools.sh"
WORKFLOWS = [ROOT / ".github" / "workflows" / name for name in ("rlm.yml", "realism.yml")]


def test_the_pin_is_one_package_version() -> None:
    assert re.fullmatch(r"\d+\.\d+\.\d+-\d+\n", PIN.read_text(encoding="utf-8"))


def test_the_install_script_installs_exactly_the_pinned_version() -> None:
    text = SCRIPT.read_text(encoding="utf-8")
    assert "zeek.version" in text
    assert '"zeek-core=$zeek_version"' in text
    assert not re.search(r"zeek-core(?!=)", text.replace("zeek-core=$zeek_version", ""))


def test_both_workflows_install_through_the_script_and_name_no_version() -> None:
    for workflow in WORKFLOWS:
        text = workflow.read_text(encoding="utf-8")
        assert "realism/ci/install-tools.sh" in text, workflow.name
        assert "zeek-core" not in text, workflow.name
        assert not re.search(r"zeek[-_ ]?(core)?[=@ ]v?\d+\.\d+", text, re.IGNORECASE), (
            workflow.name
        )
