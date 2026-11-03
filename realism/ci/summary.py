# SPDX-License-Identifier: GPL-3.0-only
"""Print the result of one module realism report as Markdown table rows.

    python3 -m realism.ci.summary MODULE REPORT

The script prints one row per check and application, and one row with the
parsers that ran. When ``GITHUB_STEP_SUMMARY`` names a file, it appends the
table with its header to that file, so the job summary of the workflow shows
every module.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any

HEADER = "| Module | Check | Application | Result | Detail |\n|---|---|---|---|---|\n"


def _verdict(passed: object) -> str:
    return "pass" if passed is True else "fail"


def rows(module: str, report: dict[str, Any]) -> list[str]:
    """Return the table rows of one realism report."""
    out = []
    for check in report.get("checks", []):
        kind = check.get("check", "unknown")
        for app in check.get("applications", []):
            if kind == "conformance":
                detail = (
                    f"{app['conforming']} of {app['sessions']} sessions conform, "
                    f"observed {json.dumps(app['observed'], sort_keys=True)}"
                )
            elif kind == "parser_errors":
                detail = (
                    f"{app['parser_errors']} parser errors, "
                    f"{app['malformed_packets']} malformed packets"
                )
            else:
                detail = ""
            # Parser error entries carry counts and no verdict of their own.
            passed = app.get("passed", app.get("parser_errors") == 0)
            out.append(
                f"| {module} | {kind} | {app['application']} | {_verdict(passed)} | {detail} |"
            )
        if kind == "parser_errors":
            ran = ", ".join(f"{p['parser']} {p['version']}" for p in check.get("parsers", []))
            missing = ", ".join(check.get("missing_parsers", []))
            detail = f"ran {ran or 'none'}" + (f", missing {missing}" if missing else "")
            out.append(f"| {module} | {kind} | all | {_verdict(check['passed'])} | {detail} |")
    return out


def main(argv: list[str] | None = None) -> int:
    """Print and append the rows of the report named by the arguments."""
    args = sys.argv[1:] if argv is None else argv
    if len(args) != 2:
        print("usage: python3 -m realism.ci.summary MODULE REPORT", file=sys.stderr)
        return 2
    module, path = args
    try:
        report = json.loads(Path(path).read_text(encoding="utf-8"))
        table = rows(module, report)
    except (OSError, ValueError, KeyError, TypeError) as exc:
        print(f"summary failed: {path}: {exc}", file=sys.stderr)
        return 2
    print("\n".join(table))
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(HEADER + "\n".join(table) + "\n\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
