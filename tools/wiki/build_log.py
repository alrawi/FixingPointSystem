#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Assemble docs/wiki/Log.md from the entry files in docs/wiki/log/.

Each wiki operation adds one file docs/wiki/log/<YYYY-MM-DD>T<HHMM>-<op>-<slug>.md
whose first line is "## [YYYY-MM-DD] <op> | <title>". Parallel pull requests add
different files, so they never conflict. The published Log page concatenates the
entries in file name order, which is chronological.

Usage: build_log.py [--out FILE] [--check]
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LOG_DIR = ROOT / "docs" / "wiki" / "log"
NAME_RE = re.compile(r"^(\d{4}-\d{2}-\d{2})T(\d{4})-([a-z]+)-[a-z0-9-]+\.md$")
HEAD_RE = re.compile(r"^## \[(\d{4}-\d{2}-\d{2})\] ([a-z]+) \| \S.*$")
OPERATIONS = {"ingest", "query", "lint", "decision", "gate", "plan"}
INTRO = (
    "# Log\n\n"
    "This page is the append only record of wiki operations. [Wiki Maintenance]"
    "(Wiki-Maintenance.md) defines the entry format. The publish workflow builds this "
    "page from the files in `docs/wiki/log/`, one file per entry, oldest first.\n"
)


def check_entries(log_dir: Path = LOG_DIR) -> list[str]:
    """Return one message per malformed entry file."""
    errors: list[str] = []
    for path in sorted(log_dir.glob("*.md")):
        name = NAME_RE.match(path.name)
        if not name:
            errors.append(f"{path.name}: name must be YYYY-MM-DDTHHMM-<op>-<slug>.md")
            continue
        first = path.read_text(encoding="utf-8").splitlines()[:1]
        head = HEAD_RE.match(first[0]) if first else None
        if not head:
            errors.append(f"{path.name}: first line must be '## [YYYY-MM-DD] <op> | <title>'")
            continue
        if head.group(1) != name.group(1) or head.group(2) != name.group(3):
            errors.append(f"{path.name}: date or operation differs from the file name")
        if head.group(2) not in OPERATIONS:
            errors.append(f"{path.name}: unknown operation {head.group(2)}")
    return errors


def build(log_dir: Path = LOG_DIR) -> str:
    """Return the full Log page."""
    entries = [p.read_text(encoding="utf-8").strip() for p in sorted(log_dir.glob("*.md"))]
    return INTRO + "".join(f"\n{e}\n" for e in entries)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", type=Path, help="write the page here; default stdout")
    parser.add_argument("--check", action="store_true", help="only validate entry files")
    args = parser.parse_args(argv)
    errors = check_entries()
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    if args.check:
        return 0
    page = build()
    if args.out:
        args.out.write_text(page, encoding="utf-8")
    else:
        sys.stdout.write(page)
    return 0


if __name__ == "__main__":
    sys.exit(main())
