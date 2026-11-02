# SPDX-License-Identifier: GPL-3.0-only
"""Tests for the agent plan checker. Run with: python3 -m pytest tools/contracts/test_plans.py"""

from __future__ import annotations

import copy
import json
import shutil
import sys
from pathlib import Path

import pytest

pytest.importorskip("google.protobuf")
pytest.importorskip("grpc_tools")

sys.path.insert(0, str(Path(__file__).parent))
import check_plans as cp

EXAMPLE = cp.PLANS / "http_dns_smoke.json"
MODULES = cp.v.EXAMPLES / "modules"


@pytest.fixture()
def plan() -> dict:
    return json.loads(EXAMPLE.read_text(encoding="utf-8"))


def errors_for(doc: dict, tmp_path: Path, modules: Path | None = MODULES) -> list[str]:
    path = tmp_path / "plan.json"
    path.write_text(json.dumps(doc), encoding="utf-8")
    report = cp.v.Report()
    cp.check_plan_file(path, report, modules)
    return [f.message for f in report.errors]


def test_examples_pass() -> None:
    assert cp.main([]) == 0


def test_example_is_canonical(plan: dict) -> None:
    assert cp.canonical(cp.parse_plan(json.dumps(plan))) == plan


def test_unknown_field_fails(plan: dict, tmp_path: Path) -> None:
    plan["workers"][0]["threads"] = 4
    assert any(
        "does not match fps.agent.v1.Plan" in m for m in errors_for(plan, tmp_path)
    )


def test_wrong_enum_fails(plan: dict, tmp_path: Path) -> None:
    plan["topology"] = "TOPOLOGY_MESH"
    assert any(
        "does not match fps.agent.v1.Plan" in m for m in errors_for(plan, tmp_path)
    )


def test_snake_case_names_are_not_canonical(plan: dict, tmp_path: Path) -> None:
    plan["plan_id"] = plan.pop("planId")
    assert any("canonical form" in m for m in errors_for(plan, tmp_path))


def test_default_values_are_not_canonical(plan: dict, tmp_path: Path) -> None:
    plan["workers"][0]["hosts"][0]["offset"] = 0
    assert any("canonical form" in m for m in errors_for(plan, tmp_path))


def test_missing_seed_fails(plan: dict, tmp_path: Path) -> None:
    del plan["seed"]
    assert any("seed is required" in m for m in errors_for(plan, tmp_path))


def test_module_digest_mismatch_fails(plan: dict, tmp_path: Path) -> None:
    mods = tmp_path / "modules"
    shutil.copytree(MODULES, mods)
    (mods / "dns" / "README.md").write_text("changed\n", encoding="utf-8")
    assert any("module dns has digest" in m for m in errors_for(plan, tmp_path, mods))


def test_module_digest_ignores_file_order(tmp_path: Path) -> None:
    a, b = tmp_path / "a", tmp_path / "b"
    for root, order in ((a, ["x", "y/z"]), (b, ["y/z", "x"])):
        for name in order:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(name, encoding="utf-8")
    assert cp.module_tree_digest(a) == cp.module_tree_digest(b)


def test_module_digest_rejects_symlinks(tmp_path: Path) -> None:
    (tmp_path / "real").write_text("x", encoding="utf-8")
    (tmp_path / "link").symlink_to(tmp_path / "real")
    with pytest.raises(ValueError, match="symbolic link"):
        cp.module_tree_digest(tmp_path)


def test_scenario_digest_mismatch_fails(plan: dict, tmp_path: Path) -> None:
    plan["scenarios"][0]["source"] += "# edited\n"
    assert any(
        "digest does not match the source" in m for m in errors_for(plan, tmp_path)
    )


def test_invalid_scenario_source_fails(plan: dict, tmp_path: Path) -> None:
    source = plan["scenarios"][0]["source"].replace("min_bytes: 12", "min_bytes: -1", 1)
    plan["scenarios"][0]["source"] = source
    plan["scenarios"][0]["digest"] = cp.sha256_digest(source.encode())
    assert any("/source" in f for f in _paths(plan, tmp_path))


def _paths(doc: dict, tmp_path: Path) -> list[str]:
    path = tmp_path / "plan.json"
    path.write_text(json.dumps(doc), encoding="utf-8")
    report = cp.v.Report()
    cp.check_plan_file(path, report, MODULES)
    return [f.path for f in report.errors]


def test_unknown_mix_entry_fails(plan: dict, tmp_path: Path) -> None:
    plan["workers"][1]["rates"][0]["mixEntry"] = "video"
    assert any("unknown mix entry 'video'" in m for m in errors_for(plan, tmp_path))


def test_weights_must_sum_to_one(plan: dict, tmp_path: Path) -> None:
    plan["mix"][0]["weight"] = 0.5
    assert any("sum to 1" in m for m in errors_for(plan, tmp_path))


def test_overlapping_host_ranges_fail(plan: dict, tmp_path: Path) -> None:
    plan["workers"][1]["hosts"][0]["offset"] = 100
    assert any("overlaps worker 0" in m for m in errors_for(plan, tmp_path))


def test_host_range_outside_slice_fails(plan: dict, tmp_path: Path) -> None:
    plan["workers"][1]["hosts"][0]["count"] = 200
    assert any("outside the slice" in m for m in errors_for(plan, tmp_path))


def test_pool_larger_than_prefix_fails(plan: dict, tmp_path: Path) -> None:
    plan["hostPools"][0]["poolCount"] = 300
    assert any("usable addresses" in m for m in errors_for(plan, tmp_path))


def test_rate_kind_must_match_objective(plan: dict, tmp_path: Path) -> None:
    plan["load"]["objective"] = "LOAD_OBJECTIVE_CONCURRENT_SESSIONS"
    assert any("needs concurrentSessions" in m for m in errors_for(plan, tmp_path))


def test_hold_phase_needs_equal_fractions(plan: dict, tmp_path: Path) -> None:
    plan["load"]["phases"][1]["endFraction"] = 0.5
    assert any("equal start and end" in m for m in errors_for(plan, tmp_path))


def test_app_sim_needs_both_roles(plan: dict, tmp_path: Path) -> None:
    doc = copy.deepcopy(plan)
    doc["hostPools"] = [p for p in doc["hostPools"] if p["role"] == "HOST_ROLE_CLIENT"]
    assert any("app_sim needs" in m for m in errors_for(doc, tmp_path))
