# SPDX-License-Identifier: GPL-3.0-only
"""Match, mismatch, and update tests for the golden file fixture."""

from __future__ import annotations

import os
from pathlib import Path

import pytest
from fps_golden import Golden, GoldenError, Outcome, check, diff, update_mode


def test_update_mode_only_for_one() -> None:
    assert update_mode({"UPDATE_GOLDEN": "1"})
    for value in ["", "0", "true", "yes"]:
        assert not update_mode({"UPDATE_GOLDEN": value}), value
    assert not update_mode({})


def test_match_passes_against_committed_testdata(golden: Golden) -> None:
    assert golden.check("report.txt", "sessions 4\nerrors 0\n") is Outcome.MATCHED


def test_match_leaves_the_file_untouched_in_update_mode(tmp_path: Path) -> None:
    path = tmp_path / "out.txt"
    path.write_text("same\n")
    os.utime(path, (1_000_000, 1_000_000))
    assert check(path, "same\n", update=True) is Outcome.MATCHED
    assert path.stat().st_mtime == 1_000_000


def test_mismatch_names_the_golden_file_and_shows_a_diff(tmp_path: Path) -> None:
    path = tmp_path / "out.txt"
    path.write_text("sessions 4\nerrors 0\n")
    with pytest.raises(GoldenError) as info:
        check(path, "sessions 4\nerrors 1\n", update=False)
    msg = str(info.value)
    assert info.value.path == path
    assert f"golden file {path}" in msg
    assert "UPDATE_GOLDEN=1" in msg
    assert "-errors 0\n+errors 1\n" in msg
    assert path.read_text() == "sessions 4\nerrors 0\n"


def test_mismatch_fails_the_test_through_the_fixture(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("UPDATE_GOLDEN", raising=False)
    (tmp_path / "out.txt").write_text("a\n")
    helper = Golden(tmp_path, update_mode())
    with pytest.raises(pytest.fail.Exception) as info:
        helper.check("out.txt", "b\n")
    assert str(tmp_path / "out.txt") in str(info.value)
    assert "-a\n+b\n" in str(info.value)


def test_missing_file_fails_without_update(tmp_path: Path) -> None:
    path = tmp_path / "out.txt"
    with pytest.raises(GoldenError, match="does not exist"):
        check(path, "x\n", update=False)
    assert not path.exists()


def test_update_rewrites_a_differing_file(tmp_path: Path) -> None:
    path = tmp_path / "out.txt"
    path.write_text("old\n")
    assert check(path, "new\n", update=True) is Outcome.WRITTEN
    assert path.read_text() == "new\n"
    assert check(path, "new\n", update=False) is Outcome.MATCHED


def test_update_through_the_environment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("UPDATE_GOLDEN", "1")
    helper = Golden(tmp_path / "nested" / "dir", update_mode())
    assert helper.check("out.bin", b"\x00\x01\x02") is Outcome.WRITTEN
    assert (tmp_path / "nested" / "dir" / "out.bin").read_bytes() == b"\x00\x01\x02"


def test_diff_marks_missing_final_newline() -> None:
    assert "-a\n+a\n\\ No newline at end of file\n" in diff("f", b"a\n", b"a")


def test_diff_summarizes_binary_content() -> None:
    d = diff("f.bin", b"\x00\x01\x02\xff", b"\x00\x01\x03\xff\x04")
    assert "golden has 4 bytes, actual has 5 bytes" in d
    assert "first difference at offset 2" in d
    assert "\n-02 ff\n+03 ff 04\n" in d
