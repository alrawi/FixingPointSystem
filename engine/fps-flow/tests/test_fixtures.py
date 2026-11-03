# SPDX-License-Identifier: GPL-3.0-only
"""Python half of the loader parity test of ENG-02.

The Rust test tests/fixtures.rs and this test read the same fixture modules
under fixtures/modules and the same verdicts in fixtures/verdicts.yaml. This
test runs the semantic checks of tools/contracts/validate.py on every fixture,
so each verdict holds in both validators.
"""
from __future__ import annotations

import sys
from pathlib import Path

import pytest
import yaml

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures"
MODULES = FIXTURES / "modules"
REPO = HERE.parents[2]
sys.path.insert(0, str(REPO / "tools" / "contracts"))

import validate  # noqa: E402

CASES = yaml.safe_load((FIXTURES / "verdicts.yaml").read_text(encoding="utf-8"))["cases"]


@pytest.fixture(scope="module")
def report() -> validate.Report:
    report = validate.Report()
    index = validate.ModuleIndex()
    for mod_dir in sorted(p for p in MODULES.iterdir() if p.is_dir()):
        validate.check_module_dir(mod_dir, report, index)
    validate.check_index_scenarios(index, report)
    return report


def findings(report: validate.Report, scenario: str, severity: str = "error") -> list[validate.Finding]:
    file = validate._rel(MODULES / scenario)
    return [f for f in report.findings if f.file == file and f.severity == severity]


def test_manifests_are_valid(report: validate.Report) -> None:
    bad = [f for f in report.errors if f.file.endswith("manifest.yaml")]
    assert not bad, "\n".join(map(str, bad))


def test_every_fixture_has_a_verdict() -> None:
    listed = {c["scenario"] for c in CASES}
    on_disk = {str(p.relative_to(MODULES)) for p in MODULES.glob("*/scenarios/*.yaml")}
    assert listed == on_disk


@pytest.mark.parametrize("case", CASES, ids=[c["scenario"] for c in CASES])
def test_python_verdict(report: validate.Report, case: dict) -> None:
    errors = findings(report, case["scenario"])
    if case["verdict"] == "accept":
        assert not errors, "\n".join(map(str, errors))
        return
    assert errors, f"{case['scenario']} passed but the verdict is reject"
    if "pointer" in case:
        assert case["pointer"] in [f.path for f in errors], "\n".join(map(str, errors))


def test_unchecked_module_is_a_warning(report: validate.Report) -> None:
    warnings = findings(report, "fx/scenarios/unchecked_module.yaml", "warning")
    assert any("ghost" in f.message for f in warnings)
