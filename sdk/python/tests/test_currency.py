# SPDX-License-Identifier: GPL-3.0-only
"""The committed client must match the OpenAPI document."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

from fixingpoint import _generated

SDK = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "generate_client", SDK / "scripts" / "generate_client.py"
)
assert spec and spec.loader
gen = importlib.util.module_from_spec(spec)
sys.modules["generate_client"] = gen
spec.loader.exec_module(gen)

OUTPUT = SDK / "fixingpoint" / "_generated.py"


def test_committed_client_is_current() -> None:
    assert gen.main(["--check"]) == 0, (
        "fixingpoint/_generated.py drifted from contracts/openapi/fps-api-v1.yaml. "
        "Run python3 sdk/python/scripts/generate_client.py and commit the result."
    )


def test_generator_output_is_deterministic() -> None:
    assert gen.render(gen.DEFAULT_SPEC) == gen.render(gen.DEFAULT_SPEC)


def test_recorded_digest_matches_the_contract() -> None:
    import hashlib

    digest = hashlib.sha256(gen.DEFAULT_SPEC.read_bytes()).hexdigest()
    assert digest == _generated.SOURCE_SHA256


def test_check_fails_when_the_contract_gains_an_operation(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    drifted = tmp_path / "api.yaml"
    text = gen.DEFAULT_SPEC.read_text(encoding="utf-8")
    drifted.write_text(
        text.replace(
            "  /agents:\n",
            "  /ping:\n    get:\n      operationId: ping\n"
            '      responses:\n        "204": { description: Pong. }\n  /agents:\n',
            1,
        ),
        encoding="utf-8",
    )
    assert "operationId: ping" in drifted.read_text(encoding="utf-8")
    assert gen.main(["--check", "--spec", str(drifted)]) == 1
    assert "out of date" in capsys.readouterr().err


def test_check_fails_when_a_schema_changes(tmp_path: Path) -> None:
    drifted = tmp_path / "api.yaml"
    text = gen.DEFAULT_SPEC.read_text(encoding="utf-8")
    changed = text.replace("enum: [pending, preparing,", "enum: [queued, pending, preparing,", 1)
    assert changed != text
    drifted.write_text(changed, encoding="utf-8")
    assert gen.main(["--check", "--spec", str(drifted)]) == 1


def test_check_fails_for_a_hand_edited_client(tmp_path: Path) -> None:
    edited = tmp_path / "_generated.py"
    edited.write_text(OUTPUT.read_text(encoding="utf-8") + "\n# edit\n", encoding="utf-8")
    assert gen.main(["--check", "--output", str(edited)]) == 1
    assert gen.main(["--check", "--output", str(tmp_path / "missing.py")]) == 1


def test_generate_writes_the_file_that_check_accepts(tmp_path: Path) -> None:
    target = tmp_path / "_generated.py"
    assert gen.main(["--output", str(target)]) == 0
    assert target.read_text(encoding="utf-8") == OUTPUT.read_text(encoding="utf-8")
    assert gen.main(["--check", "--output", str(target)]) == 0
