# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of make_plan.py, which need no root and no Rust toolchain."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT))

import make_plan

from realism import labels as realism_labels


@pytest.fixture(scope="module")
def built(tmp_path_factory: pytest.TempPathFactory) -> Path:
    out = tmp_path_factory.mktemp("plan")
    make_plan.build(ROOT, out, 20, 42, "dpc", "dps", True)
    return out


def test_every_scenario_is_a_mix_entry_on_its_own_server_port(built: Path) -> None:
    plan = json.loads((built / "plan.json").read_text(encoding="utf-8"))
    assert plan["backend"] == "BACKEND_AF_XDP"
    assert len(plan["mix"]) == 10
    ports = [
        e["params"].get("server_port")
        for e in plan["mix"]
        if e["id"].startswith("https_")
    ]
    assert ports == [None] * 4, "the https scenarios keep the ports of their manifests"
    plain = [
        e["params"]["server_port"]
        for e in plan["mix"]
        if not e["id"].startswith("https_")
    ]
    assert plain == list(range(8080, 8086))


def test_the_module_tree_digest_covers_only_the_staged_files(built: Path) -> None:
    plan = json.loads((built / "plan.json").read_text(encoding="utf-8"))
    tree = built / "modules" / "http1"
    assert sorted(p.name for p in tree.iterdir()) == ["manifest.yaml", "scenarios"]
    assert plan["modules"][0]["digest"] == make_plan.tree_digest(tree)
    assert plan["modules"][0]["digest"].startswith("sha256:")


def test_the_labels_load_and_cover_every_port(built: Path) -> None:
    labels = realism_labels.load(built / "labels.yaml")
    assert labels.tls is not None
    text = (built / "labels.yaml").read_text(encoding="utf-8")
    for port in [*range(8080, 8086), *range(8443, 8447)]:
        assert f"server_port: {port}" in text


def test_without_keys_the_labels_leave_the_encryption_check_out(tmp_path: Path) -> None:
    make_plan.build(ROOT, tmp_path, 20, 42, "dpc", "dps", False)
    labels = realism_labels.load(tmp_path / "labels.yaml")
    assert labels.tls is None
    assert "tls_profile" not in (tmp_path / "labels.yaml").read_text(encoding="utf-8")
