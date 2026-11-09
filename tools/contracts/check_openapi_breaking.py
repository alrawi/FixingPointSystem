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

OASDIFF_BREAKING_FOUND = 1


def breaking_exit_code(base: str, revision: str) -> tuple[int, str]:
    """Run `oasdiff breaking` and return its exit code and output."""
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
