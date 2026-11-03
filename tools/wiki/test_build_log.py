# SPDX-License-Identifier: GPL-3.0-only
"""Tests for tools/wiki/build_log.py."""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import build_log  # noqa: E402


def _entry(d: Path, name: str, head: str) -> None:
    (d / name).write_text(f"{head}\n\nBody.\n", encoding="utf-8")


def test_entries_build_in_file_name_order(tmp_path: Path) -> None:
    _entry(tmp_path, "2026-11-03T1200-ingest-b.md", "## [2026-11-03] ingest | B")
    _entry(tmp_path, "2026-11-03T0900-plan-a.md", "## [2026-11-03] plan | A")
    page = build_log.build(tmp_path)
    assert page.index("| A") < page.index("| B")
    assert build_log.check_entries(tmp_path) == []


def test_mismatched_date_fails(tmp_path: Path) -> None:
    _entry(tmp_path, "2026-11-03T0900-plan-a.md", "## [2026-11-04] plan | A")
    assert build_log.check_entries(tmp_path)


def test_unknown_operation_fails(tmp_path: Path) -> None:
    _entry(tmp_path, "2026-11-03T0900-chat-a.md", "## [2026-11-03] chat | A")
    assert any("unknown operation" in e for e in build_log.check_entries(tmp_path))


def test_repository_log_is_valid() -> None:
    assert build_log.check_entries() == []
