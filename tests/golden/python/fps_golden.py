# SPDX-License-Identifier: GPL-3.0-only
"""Golden file comparison for pytest.

A test requests the ``golden`` fixture and calls ``golden.check(name, output)``.
The fixture compares the output with ``testdata/<name>`` next to the test file and
fails the test with a unified diff that names the golden file on mismatch. It
rewrites the file only when the environment variable ``UPDATE_GOLDEN`` equals ``1``.
"""

from __future__ import annotations

import difflib
import enum
import os
from collections.abc import Mapping
from pathlib import Path

import pytest

UPDATE_ENV = "UPDATE_GOLDEN"


class Outcome(enum.Enum):
    """The result of a comparison that did not fail."""

    MATCHED = "matched"
    WRITTEN = "written"


class GoldenError(AssertionError):
    """The output differs from the golden file, or the golden file is missing."""

    def __init__(self, path: Path, message: str) -> None:
        super().__init__(message)
        self.path = path


def update_mode(environ: Mapping[str, str] | None = None) -> bool:
    """Return True only when UPDATE_GOLDEN equals 1, so a typo never rewrites files."""
    env = os.environ if environ is None else environ
    return env.get(UPDATE_ENV) == "1"


def check(path: Path, actual: str | bytes, update: bool) -> Outcome:
    """Compare ``actual`` with the golden file at ``path``.

    With ``update`` set, the function writes the file and its missing parent
    directories only when the content differs, so unchanged files keep their
    modification time. Text is written as UTF-8 without newline translation.
    """
    data = actual.encode() if isinstance(actual, str) else actual
    try:
        golden: bytes | None = path.read_bytes()
    except FileNotFoundError:
        golden = None
    if golden == data:
        return Outcome.MATCHED
    if update:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return Outcome.WRITTEN
    if golden is None:
        raise GoldenError(
            path,
            f"golden file {path} does not exist; run with {UPDATE_ENV}=1 to create it",
        )
    raise GoldenError(
        path,
        f"golden file {path} does not match the output; run with {UPDATE_ENV}=1 to accept "
        f"the output and review the change with git diff\n{diff(str(path), golden, data)}",
    )


def diff(label: str, golden: bytes, actual: bytes) -> str:
    """Return a unified diff from ``golden`` to ``actual`` with a header that names ``label``.

    Content that is not UTF-8 gets a summary of the first differing byte instead.
    """
    try:
        g = golden.decode()
        a = actual.decode()
    except UnicodeDecodeError:
        return _binary_diff(label, golden, actual)
    lines = difflib.unified_diff(
        g.splitlines(keepends=True),
        a.splitlines(keepends=True),
        fromfile=f"golden {label}",
        tofile="actual",
    )
    out = []
    for line in lines:
        if line.startswith(("---", "+++")):
            line = line.rstrip() + "\n"
        out.append(
            line if line.endswith("\n") else line + "\n\\ No newline at end of file\n"
        )
    return "".join(out)


def _binary_diff(label: str, golden: bytes, actual: bytes) -> str:
    offset = next(
        (i for i, (x, y) in enumerate(zip(golden, actual)) if x != y),
        min(len(golden), len(actual)),
    )

    def window(b: bytes) -> str:
        return " ".join(f"{x:02x}" for x in b[offset : offset + 16])

    return (
        f"--- golden {label}\n+++ actual\n"
        f"binary content differs: golden has {len(golden)} bytes, actual has {len(actual)} "
        f"bytes, first difference at offset {offset}\n-{window(golden)}\n+{window(actual)}\n"
    )


class Golden:
    """Compares output with golden files under one testdata directory."""

    def __init__(self, testdata: Path, update: bool) -> None:
        self.testdata = testdata
        self.update = update

    def path(self, name: str) -> Path:
        """Return the golden file path for ``name``."""
        return self.testdata / name

    def check(self, name: str, actual: str | bytes) -> Outcome:
        """Compare ``actual`` with ``testdata/<name>`` and fail the test on mismatch."""
        try:
            return check(self.path(name), actual, self.update)
        except GoldenError as e:
            pytest.fail(str(e), pytrace=False)


@pytest.fixture
def golden(request: pytest.FixtureRequest) -> Golden:
    """Return a Golden helper for the testdata directory next to the test file."""
    return Golden(request.path.parent / "testdata", update_mode())
