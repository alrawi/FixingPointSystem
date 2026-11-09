# SPDX-License-Identifier: GPL-3.0-only
"""Fixture tests of the OpenAPI breaking check (CON-16).

They need the oasdiff binary. Without it the tests are skipped, unless the
variable FPS_REQUIRE_OASDIFF is set, as in .github/workflows/openapi-breaking.yml,
where a missing binary is a failure.
"""
from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).parent))
import check_openapi_breaking as c

FIXTURES = Path(__file__).parent / "fixtures" / "openapi-breaking"
SPEC = Path(__file__).resolve().parents[2] / "contracts" / "openapi"

if shutil.which("oasdiff") is None and not os.environ.get("FPS_REQUIRE_OASDIFF"):
    pytest.skip("oasdiff is not installed", allow_module_level=True)


def test_additive_change_passes() -> None:
    code, out = c.breaking_exit_code(str(FIXTURES / "base.yaml"), str(FIXTURES / "additive.yaml"))
    assert code == 0, out


def test_breaking_change_fails() -> None:
    code, out = c.breaking_exit_code(str(FIXTURES / "base.yaml"), str(FIXTURES / "breaking.yaml"))
    assert code == c.OASDIFF_BREAKING_FOUND, out
    for rule in ("removed", "new-required-request-parameter", "required-property-removed"):
        assert rule in out, f"oasdiff did not report {rule}:\n{out}"


@pytest.mark.parametrize("name", ["fps-api-v1.yaml", "fps-metrics.yaml"])
def test_contract_against_itself_passes(name: str) -> None:
    code, out = c.breaking_exit_code(str(SPEC / name), str(SPEC / name))
    assert code == 0, out


def test_check_script_exit_codes() -> None:
    base, bad = str(FIXTURES / "base.yaml"), str(FIXTURES / "breaking.yaml")
    assert c.main(["x", base, base]) == 0
    assert c.main(["x", base, bad]) == 1
    assert c.main(["x", base, str(FIXTURES / "absent.yaml")]) == 2
