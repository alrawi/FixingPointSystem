# SPDX-License-Identifier: GPL-3.0-only
"""Compare two OpenAPI documents with oasdiff and fail on breaking changes.

    python3 tools/contracts/check_openapi_breaking.py BASE REVISION

BASE is the document of the base branch and REVISION the one of the change.
The exit code is 0 when oasdiff finds no breaking change, 1 when it finds at
least one error level change, and 2 when oasdiff cannot run or cannot read a
document. `breaking_exit_code` is also what the fixture test uses to prove that
the check fails on a deliberately breaking change.
"""
from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

import yaml

OASDIFF_BREAKING_FOUND = 1


def _opaque(node: Any) -> Any:
    """Replace references to JSON Schema files by an opaque object schema.

    The test configuration schema is JSON Schema 2020-12 (numeric
    `exclusiveMinimum`), which oasdiff cannot load as an OpenAPI schema. The
    schema has its own contract version, so the REST check treats it as an
    opaque object.
    """
    if isinstance(node, dict):
        ref = node.get("$ref")
        if isinstance(ref, str) and ref.startswith("../") and ref.split("#")[0].endswith(".json"):
            return {"type": "object"}
        return {k: _opaque(v) for k, v in node.items()}
    if isinstance(node, list):
        return [_opaque(v) for v in node]
    return node


def sanitized(path: str, into: Path) -> str:
    """Write a copy of every YAML document next to `path` into `into` and return the copy of `path`.

    Relative references between the documents (fps-metrics.yaml points into
    fps-api-v1.yaml) keep working, because the copies stay side by side.
    """
    src = Path(path)
    for doc in src.parent.glob("*.yaml"):
        data = yaml.safe_load(doc.read_text(encoding="utf-8"))
        (into / doc.name).write_text(yaml.safe_dump(_opaque(data), sort_keys=False), encoding="utf-8")
    return str(into / src.name)


def breaking_exit_code(base: str, revision: str) -> tuple[int, str]:
    """Run `oasdiff breaking` and return its exit code and output."""
    with tempfile.TemporaryDirectory() as tmp:
        (Path(tmp) / "base").mkdir()
        (Path(tmp) / "rev").mkdir()
        try:
            base_copy = sanitized(base, Path(tmp) / "base")
            rev_copy = sanitized(revision, Path(tmp) / "rev")
        except (OSError, yaml.YAMLError) as err:
            return 102, f"cannot read the documents: {err}\n"
        return _run(base_copy, rev_copy)


def _run(base: str, revision: str) -> tuple[int, str]:
    proc = subprocess.run(
        ["oasdiff", "breaking", base, revision, "--fail-on", "ERR"],
        capture_output=True,
        text=True,
        check=False,
    )
    return proc.returncode, proc.stdout + proc.stderr


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        code, output = breaking_exit_code(argv[1], argv[2])
    except FileNotFoundError:
        print("oasdiff is not installed", file=sys.stderr)
        return 2
    print(output, end="")
    if code == 0:
        print("No breaking change in the OpenAPI document.")
        return 0
    if code == OASDIFF_BREAKING_FOUND:
        print("The OpenAPI document has breaking changes (see above).", file=sys.stderr)
        return 1
    print(f"oasdiff failed with exit code {code}.", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
