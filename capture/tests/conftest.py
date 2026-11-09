# SPDX-License-Identifier: GPL-3.0-only
"""Fixtures that write synthetic captures and extract sessions from them."""

from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from capture.cli import extract

SCHEMA = json.loads((Path(__file__).parent.parent / "sessions.schema.json").read_text())

Extract = Callable[..., dict[str, Any]]


@pytest.fixture
def run(tmp_path: Path) -> Extract:
    """Write capture bytes to a file, extract sessions, and check the schema."""

    def go(data: bytes, **kwargs: Any) -> dict[str, Any]:
        path = tmp_path / "in.cap"
        path.write_bytes(data)
        doc = extract(path, **kwargs)
        jsonschema.validate(json.loads(json.dumps(doc)), SCHEMA)
        return doc

    return go
