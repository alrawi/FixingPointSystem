#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Check that Zeek parses the control and data connections of the ftp fixture.

The acceptance of MOD-10 asks for ftp.log and files.log without errors where
the realism job runs Zeek. The realism runner already fails a capture with any
weird of a parser error kind. This check adds what the runner does not read:

* ``ftp.log`` holds one row for every RETR and every STOR, with a user name, a
  file name, a reply code below 400, a passive data channel, and the
  identifier of the file,
* ``files.log`` holds a file of the source FTP_DATA with bytes for every
  RETR and every STOR, and none of those files lost bytes, and
* ``weird.log`` holds no entry whose name starts with ``ftp``.

The check does not count the rows of LIST, because Zeek versions differ in
whether they log the commands that list a directory. It reports the count.

Usage::

    zeek_check.py CAPTURE --file-transfers N [--zeek PROGRAM]

``--file-transfers`` is the number of RETR and STOR commands in the capture.
The script exits with 0 when every check passes, 1 when a check fails, and 2
when Zeek cannot run.
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
FILE_COMMANDS = ("RETR", "STOR")


def read_log(path: Path) -> list[dict]:
    """Return the rows of a JSON Zeek log, or an empty list when it is missing."""
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return []
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def check(logs: dict[str, list[dict]], file_transfers: int) -> list[str]:
    """Return one line for each failed check, or an empty list when all pass."""
    errors: list[str] = []
    ftp = [r for r in logs.get("ftp", []) if r.get("command") in FILE_COMMANDS]
    files = [f for f in logs.get("files", []) if "FTP_DATA" in as_list(f.get("source"))]
    weird = [w for w in logs.get("weird", []) if str(w.get("name", "")).lower().startswith("ftp")]

    for w in weird:
        errors.append(f"weird.log: {w.get('name')} on {w.get('id.orig_h')} to {w.get('id.resp_h')}")

    if len(ftp) != file_transfers:
        errors.append(f"ftp.log: {len(ftp)} RETR and STOR rows, expected {file_transfers}")
    for i, row in enumerate(ftp):
        what = f"ftp.log {row.get('command')} row {i + 1}"
        for key in ("user", "arg", "fuid"):
            if not row.get(key):
                errors.append(f"{what} lacks {key}")
        code = row.get("reply_code")
        if not isinstance(code, int) or not 100 <= code < 400:
            errors.append(f"{what} has reply code {code}")
        if row.get("data_channel.passive") is not True:
            errors.append(f"{what} has no passive data channel")

    listed = {row.get("fuid") for row in ftp}
    carried = [f for f in files if f.get("fuid") in listed]
    if len(carried) != file_transfers:
        errors.append(f"files.log: {len(carried)} FTP_DATA files of RETR and STOR, expected {file_transfers}")
    for f in carried:
        if not f.get("seen_bytes"):
            errors.append(f"files.log: {f.get('fuid')} has no bytes")
        if f.get("missing_bytes"):
            errors.append(f"files.log: {f.get('fuid')} lost {f.get('missing_bytes')} bytes")
    return errors


def as_list(value: object) -> list:
    """Return a log field as a list, because Zeek writes a set as an array."""
    if value is None:
        return []
    return value if isinstance(value, list) else [value]


def run_zeek(program: str, capture: Path, timeout_s: float = 300.0) -> dict[str, list[dict]]:
    """Run Zeek on the capture and return the rows of the logs that the check reads."""
    with tempfile.TemporaryDirectory(prefix="fps-ftp-zeek-") as tmp:
        cwd = Path(tmp)
        argv = [program, "-C", "-r", str(capture.resolve()), "LogAscii::use_json=T"]
        proc = subprocess.run(
            argv, cwd=cwd, capture_output=True, text=True, timeout=timeout_s, check=False
        )
        if proc.returncode != 0:
            raise RuntimeError(f"zeek exited with {proc.returncode}: {proc.stderr.strip()}")
        return {name: read_log(cwd / f"{name}.log") for name in ("ftp", "files", "weird")}


def main(argv: list[str] | None = None) -> int:
    """Run the check and print the result."""
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("capture", type=Path)
    p.add_argument("--file-transfers", type=int, required=True)
    p.add_argument("--zeek", default=shutil.which("zeek") or DEFAULT_ZEEK)
    args = p.parse_args(argv)
    try:
        logs = run_zeek(args.zeek, args.capture)
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"zeek_check: {exc}", file=sys.stderr)
        return 2
    errors = check(logs, args.file_transfers)
    listings = sum(1 for r in logs["ftp"] if r.get("command") in ("LIST", "NLST", "MLSD"))
    print(
        f"zeek_check: ftp.log {len(logs['ftp'])} rows ({listings} of them list a directory), "
        f"files.log {len(logs['files'])} rows, weird.log {len(logs['weird'])} rows"
    )
    for command in ("PASV", "EPSV", "RETR", "STOR"):
        sample = next((r for r in logs["ftp"] if r.get("command") == command), None)
        print(f"zeek_check: first {command} row {json.dumps(sample, sort_keys=True)}")
    sample_file = next((f for f in logs["files"] if "FTP_DATA" in as_list(f.get("source"))), None)
    print(f"zeek_check: first FTP_DATA file {json.dumps(sample_file, sort_keys=True)}")
    for e in errors:
        print(f"zeek_check: FAIL {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
