#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Check that Zeek parses every message of the smtp fixture.

The acceptance of MOD-07 says that Zeek logs every fixture message without
parser errors. The realism runner already fails a capture with any weird of a
parser error kind. This check adds what the runner does not read:

* ``smtp.log`` holds one row per message with the sender, the recipients, the
  header fields, and the identifiers of the files that the message carried,
* ``files.log`` holds one SMTP file with a name for every attachment, and the
  bytes of each file arrived whole, and
* ``weird.log`` holds no entry whose name starts with ``smtp``.

Usage::

    zeek_check.py CAPTURE --messages N --attachments M [--zeek PROGRAM]

``--messages`` is the number of messages in the capture and ``--attachments``
the number of those that carry an attachment. The script exits with 0 when
every check passes, 1 when a check fails, and 2 when Zeek cannot run.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

DEFAULT_ZEEK = "/opt/zeek/bin/zeek"
ATTACHMENT_SUFFIXES = (".png", ".pdf", ".zip", ".jpg", ".gz")


def read_log(path: Path) -> list[dict]:
    """Return the rows of a JSON Zeek log, or an empty list when it is missing."""
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return []
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def as_list(value: object) -> list:
    """Return a log field as a list, because Zeek writes a set as an array."""
    if value is None:
        return []
    return value if isinstance(value, list) else [value]


def check(logs: dict[str, list[dict]], messages: int, attachments: int) -> list[str]:
    """Return one line for each failed check, or an empty list when all pass."""
    errors: list[str] = []
    smtp = logs.get("smtp", [])
    files = [f for f in logs.get("files", []) if "SMTP" in as_list(f.get("source"))]
    weird = [w for w in logs.get("weird", []) if str(w.get("name", "")).startswith("smtp")]

    for w in weird:
        errors.append(f"weird.log: {w.get('name')} on {w.get('id.orig_h')} to {w.get('id.resp_h')}")

    if len(smtp) != messages:
        errors.append(f"smtp.log: {len(smtp)} messages, expected {messages}")
    for i, row in enumerate(smtp):
        missing = [
            k
            for k in ("helo", "mailfrom", "rcptto", "from", "to", "subject", "msg_id", "date")
            if not row.get(k)
        ]
        if missing:
            errors.append(f"smtp.log row {i + 1} lacks {', '.join(missing)}")
        if not as_list(row.get("fuids")):
            errors.append(f"smtp.log row {i + 1} lists no file")

    named = [f for f in files if str(f.get("filename", "")).lower().endswith(ATTACHMENT_SUFFIXES)]
    if len(named) != attachments:
        errors.append(f"files.log: {len(named)} named attachments, expected {attachments}")
    listed = {fuid for row in smtp for fuid in as_list(row.get("fuids"))}
    for f in named:
        if f.get("fuid") not in listed:
            errors.append(f"files.log: {f.get('filename')} is not listed in smtp.log")
        if not f.get("seen_bytes"):
            errors.append(f"files.log: {f.get('filename')} has no bytes")
        if f.get("missing_bytes"):
            errors.append(f"files.log: {f.get('filename')} lost {f.get('missing_bytes')} bytes")
    return errors


def run_zeek(program: str, capture: Path, timeout_s: float = 300.0) -> dict[str, list[dict]]:
    """Run Zeek on the capture and return the rows of the logs that the check reads."""
    with tempfile.TemporaryDirectory(prefix="fps-smtp-zeek-") as tmp:
        cwd = Path(tmp)
        argv = [program, "-C", "-r", str(capture.resolve()), "LogAscii::use_json=T"]
        proc = subprocess.run(
            argv, cwd=cwd, capture_output=True, text=True, timeout=timeout_s, check=False
        )
        if proc.returncode != 0:
            raise RuntimeError(f"zeek exited with {proc.returncode}: {proc.stderr.strip()}")
        return {name: read_log(cwd / f"{name}.log") for name in ("smtp", "files", "weird")}


def main(argv: list[str] | None = None) -> int:
    """Run the check and print the result."""
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("capture", type=Path)
    p.add_argument("--messages", type=int, required=True)
    p.add_argument("--attachments", type=int, required=True)
    p.add_argument("--zeek", default=shutil.which("zeek") or DEFAULT_ZEEK)
    args = p.parse_args(argv)
    try:
        logs = run_zeek(args.zeek, args.capture)
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"zeek_check: {exc}", file=sys.stderr)
        return 2
    errors = check(logs, args.messages, args.attachments)
    print(
        f"zeek_check: smtp.log {len(logs['smtp'])} rows, files.log {len(logs['files'])} rows, "
        f"weird.log {len(logs['weird'])} rows"
    )
    for e in errors:
        print(f"zeek_check: FAIL {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
