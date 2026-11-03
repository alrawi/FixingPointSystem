# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the module selection and the helpers of the module realism job."""

from __future__ import annotations

import io
import json
from pathlib import Path

import pytest
import yaml

from realism import cli
from realism.ci import negative_control, summary
from realism.modules import (
    SHARED_PATHS,
    Module,
    ModuleError,
    crate_name,
    discover,
    matrix,
    select,
)

REPO = Path(__file__).resolve().parents[2]

HTTP1 = Module("http1", "fps-module-http1")
DNS = Module("dns", "fps-module-dns")


def make_module(root: Path, name: str, crate: str | None = None, files: bool = True) -> Path:
    """Create a module directory with the files that the realism job needs."""
    path = root / "modules" / name
    (path / "native").mkdir(parents=True)
    (path / "fixtures").mkdir()
    (path / "native" / "Cargo.toml").write_text(
        f'[package]\nname = "{crate or "fps-module-" + name}"\nversion = "0.1.0"\n'
    )
    if files:
        (path / "fixtures" / "capture.sh").write_text("#!/bin/sh\n")
        (path / "fixtures" / "expected.yaml").write_text("labels: fps-realism-labels\n")
    return path


def test_discover_finds_modules_with_all_files(tmp_path: Path) -> None:
    make_module(tmp_path, "http1")
    make_module(tmp_path, "dns")
    make_module(tmp_path, "_template")
    make_module(tmp_path, "nofixture", files=False)
    (tmp_path / "modules" / "README.md").write_text("not a module\n")
    assert discover(tmp_path) == [DNS, HTTP1]


def test_discover_without_modules_directory(tmp_path: Path) -> None:
    assert discover(tmp_path) == []


def test_discover_reads_the_repository_modules() -> None:
    names = [module.name for module in discover(REPO)]
    assert "http1" in names
    assert "dns" in names
    assert all(not name.startswith("_") for name in names)


@pytest.mark.parametrize(
    ("text", "name"),
    [
        ('[package]\nname = "fps-module-x"\n', "fps-module-x"),
        ('[package]\nedition = "2021"\nname="a_b"  # crate\n', "a_b"),
        ('[lib]\nname = "lib_x"\n\n[package] # main\nname = "pkg"\n', "pkg"),
    ],
)
def test_crate_name_reads_the_package_table(tmp_path: Path, text: str, name: str) -> None:
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text(text)
    assert crate_name(manifest) == name


def test_crate_name_without_package_name(tmp_path: Path) -> None:
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text('[lib]\nname = "only_lib"\n')
    with pytest.raises(ModuleError, match="no package name"):
        crate_name(manifest)


def test_crate_name_of_missing_manifest(tmp_path: Path) -> None:
    with pytest.raises(ModuleError, match="read"):
        crate_name(tmp_path / "Cargo.toml")


@pytest.mark.parametrize(
    ("changed", "expected"),
    [
        (None, [DNS, HTTP1]),
        ([], []),
        (["docs/wiki/Realism.md"], []),
        (["modules/http1/native/src/message.rs"], [HTTP1]),
        (["modules/dns/fixtures/expected.yaml", "modules/http1/README.md"], [DNS, HTTP1]),
        (["modules/_template/README.md", "modules/README.md"], []),
        (["modules/other/native/src/lib.rs"], []),
        (["realism/cli.py"], [DNS, HTTP1]),
        (["engine/fps-flow/src/lib.rs"], [DNS, HTTP1]),
        (["tests/harness/netns.sh"], [DNS, HTTP1]),
        (["Cargo.lock"], [DNS, HTTP1]),
        ([".github/workflows/realism.yml"], [DNS, HTTP1]),
        (["engine.md", "realismx/a.py", "tests/harnessx"], []),
        (["  modules/dns/README.md  ", ""], [DNS]),
    ],
)
def test_select(changed: list[str] | None, expected: list[Module]) -> None:
    assert select([DNS, HTTP1], changed) == expected


def test_matrix() -> None:
    assert matrix([HTTP1]) == {"include": [{"module": "http1", "crate": "fps-module-http1"}]}
    assert matrix([]) == {"include": []}


def test_workflow_triggers_match_the_shared_paths() -> None:
    """Every shared path triggers the workflow, and so does every module directory."""
    workflow = yaml.safe_load((REPO / ".github" / "workflows" / "realism.yml").read_text())
    triggers = workflow[True]
    expected = {"modules/**"} | {
        entry + "**" if entry.endswith("/") else entry for entry in SHARED_PATHS
    }
    assert set(triggers["pull_request"]["paths"]) == expected
    assert set(triggers["push"]["paths"]) == expected


def test_cli_modules_matrix(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    make_module(tmp_path, "http1")
    make_module(tmp_path, "dns")
    changed = tmp_path / "changed.txt"
    changed.write_text("modules/http1/native/src/lib.rs\n")
    assert cli.main(["modules", "--root", str(tmp_path), "--changed", str(changed)]) == 0
    assert json.loads(capsys.readouterr().out) == matrix([HTTP1])
    monkeypatch.setattr("sys.stdin", io.StringIO("realism/report.py\n"))
    assert (
        cli.main(["modules", "--root", str(tmp_path), "--changed", "-", "--format", "names"]) == 0
    )
    assert capsys.readouterr().out == "dns\nhttp1\n"


def test_cli_modules_without_selection(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    changed = tmp_path / "changed.txt"
    changed.write_text("docs/wiki/Home.md\n")
    assert cli.main(["modules", "--root", str(REPO), "--changed", str(changed)]) == 0
    assert json.loads(capsys.readouterr().out) == {"include": []}
    assert (
        cli.main(["modules", "--root", str(REPO), "--changed", str(changed), "--format", "names"])
        == 0
    )
    assert capsys.readouterr().out == ""


def test_cli_modules_errors(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert cli.main(["modules", "--changed", str(tmp_path / "absent.txt")]) == 2
    assert "realism modules failed" in capsys.readouterr().err
    path = make_module(tmp_path, "http1")
    (path / "native" / "Cargo.toml").write_text("[workspace]\n")
    assert cli.main(["modules", "--root", str(tmp_path)]) == 2
    assert "no package name" in capsys.readouterr().err


RENDER = (
    "    pub fn render(&self, out: &mut Vec<u8>) -> Result<(), FnError> {\n"
    "        out.extend_from_slice(self.method.as_bytes());\n"
    "        out.push(b' ');\n"
    "        out.extend_from_slice(self.path.as_bytes());\n"
    "        out.push(b' ');\n"
)


def test_negative_control_removes_the_space_after_the_method() -> None:
    broken = negative_control.break_request_line(RENDER)
    assert "self.method.as_bytes());\n        out.extend_from_slice(self.path" in broken
    assert broken.count("out.push(b' ');") == 1


@pytest.mark.parametrize("source", ["fn render() {}\n", RENDER + RENDER])
def test_negative_control_needs_one_anchor(source: str) -> None:
    with pytest.raises(ValueError, match="anchor"):
        negative_control.break_request_line(source)


def test_negative_control_main(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    path = tmp_path / "message.rs"
    path.write_text(RENDER)
    assert negative_control.main([str(path)]) == 0
    assert path.read_text() == negative_control.break_request_line(RENDER)
    assert negative_control.main([str(path)]) == 2
    assert "found it 0 times" in capsys.readouterr().err
    assert negative_control.main([]) == 2
    assert negative_control.main([str(tmp_path / "absent.rs")]) == 2


REPORT = {
    "checks": [
        {
            "check": "conformance",
            "passed": False,
            "applications": [
                {
                    "application": "http1",
                    "sessions": 120,
                    "conforming": 0,
                    "passed": False,
                    "observed": {"Unknown": 120},
                }
            ],
        },
        {
            "check": "parser_errors",
            "passed": True,
            "parsers": [{"parser": "tshark", "version": "4.2.2"}],
            "missing_parsers": ["zeek"],
            "applications": [
                {
                    "application": "http1",
                    "sessions": 120,
                    "parser_errors": 0,
                    "malformed_packets": 0,
                }
            ],
        },
    ]
}


def test_summary_rows() -> None:
    assert summary.rows("http1", REPORT) == [
        "| http1 | conformance | http1 | fail | 0 of 120 sessions conform, "
        'observed {"Unknown": 120} |',
        "| http1 | parser_errors | http1 | pass | 0 parser errors, 0 malformed packets |",
        "| http1 | parser_errors | all | pass | ran tshark 4.2.2, missing zeek |",
    ]


def test_summary_rows_of_parser_errors_without_parsers() -> None:
    report = {"checks": [{"check": "parser_errors", "passed": False, "applications": []}]}
    assert summary.rows("dns", report) == ["| dns | parser_errors | all | fail | ran none |"]


def test_summary_main_appends_to_the_job_summary(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    report = tmp_path / "http1.realism.json"
    report.write_text(json.dumps(REPORT))
    job = tmp_path / "summary.md"
    monkeypatch.setenv("GITHUB_STEP_SUMMARY", str(job))
    assert summary.main(["http1", str(report)]) == 0
    assert summary.main(["http1", str(report)]) == 0
    assert capsys.readouterr().out.count("| http1 | conformance |") == 2
    assert job.read_text().count(summary.HEADER) == 2


def test_summary_main_errors(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert summary.main(["http1"]) == 2
    assert summary.main(["http1", str(tmp_path / "absent.json")]) == 2
    bad = tmp_path / "bad.json"
    bad.write_text('{"checks": [{"check": "conformance", "applications": [{}]}]}')
    assert summary.main(["http1", str(bad)]) == 2
    assert "summary failed" in capsys.readouterr().err
