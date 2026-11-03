# SPDX-License-Identifier: GPL-3.0-only
"""Tests for tools/plan/plan.py."""
from __future__ import annotations

import copy
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).parent))
import plan  # noqa: E402


@pytest.fixture(scope="module")
def catalog() -> dict:
    return plan.load()


def test_catalog_is_valid(catalog: dict) -> None:
    assert plan.check(catalog) == []


def test_generated_pages_are_current(catalog: dict) -> None:
    assert plan.render(catalog, check_only=True) == 0


def test_wiki_has_no_broken_links_or_orphans() -> None:
    assert plan.lint() == []


def _mutate(catalog: dict, wid: str, **fields: object) -> dict:
    c = copy.deepcopy(catalog)
    wp = next(w for w in c["work_packages"] if w["id"] == wid)
    wp.update(fields)
    return c


def test_unknown_dependency_fails(catalog: dict) -> None:
    errors = plan.check(_mutate(catalog, "ENG-04", depends_on=["ENG-99"]))
    assert any("unknown dependency ENG-99" in e for e in errors)


def test_cycle_fails(catalog: dict) -> None:
    errors = plan.check(_mutate(catalog, "ENG-01", depends_on=["ENG-04"]))
    assert any("cycle" in e for e in errors)


def test_later_phase_dependency_fails(catalog: dict) -> None:
    errors = plan.check(_mutate(catalog, "ENG-04", depends_on=["TRN-04"]))
    assert any("later phase" in e for e in errors)


def test_path_outside_workstream_fails(catalog: dict) -> None:
    errors = plan.check(_mutate(catalog, "ENG-04", paths=["controller/"]))
    assert any("outside workstream" in e for e in errors)


def test_prefix_must_match_workstream(catalog: dict) -> None:
    errors = plan.check(_mutate(catalog, "ENG-04", workstream="TRN"))
    assert any("prefix" in e for e in errors)


def test_labels_for_epic_and_ready(catalog: dict) -> None:
    epic = next(w for w in catalog["work_packages"] if w["size"] == "L")
    assert "needs:split" in plan.issue_labels(epic, ready=True)
    assert "status:ready" not in plan.issue_labels(epic, ready=True)
    first = next(w for w in catalog["work_packages"] if w["id"] == "OPS-01")
    assert "status:ready" in plan.issue_labels(first, ready=True)


def test_lint_reports_broken_link(tmp_path: Path) -> None:
    (tmp_path / "Home.md").write_text("[a](Missing.md)\n", encoding="utf-8")
    assert plan.lint(tmp_path) == ["Home.md: broken link to Missing.md"]


def test_batch_skips_packages_that_share_a_path(catalog: dict) -> None:
    picked, skipped = plan.batch(catalog, ["ENG-02", "ENG-03", "CTL-02", "CTL-10"], 8)
    assert "ENG-03" in picked and "CTL-02" in picked
    assert ("ENG-02", "ENG-03") in skipped
    assert ("CTL-10", "CTL-02") in skipped


def test_batch_respects_the_limit(catalog: dict) -> None:
    picked, _ = plan.batch(catalog, ["ENG-03", "TRN-01", "HST-01"], 2)
    assert len(picked) == 2


def test_paths_overlap() -> None:
    assert plan.paths_overlap("controller/", "controller/internal/api/")
    assert not plan.paths_overlap("controller/internal/api/", "controller/internal/store/")
    assert not plan.paths_overlap("engine/fps-io/", "engine/fps-io2/")


def test_log_entry_with_bad_heading_fails(tmp_path: Path) -> None:
    (tmp_path / "Home.md").write_text("# Home\n", encoding="utf-8")
    (tmp_path / "log").mkdir()
    (tmp_path / "log" / "2026-11-03T1200-ingest-x.md").write_text("# wrong\n", encoding="utf-8")
    assert any("first line" in e for e in plan.lint(tmp_path))
