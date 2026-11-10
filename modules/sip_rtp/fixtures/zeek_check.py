#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Check that Zeek logs every call of the sip_rtp fixture.

The acceptance of MOD-14 asks that Zeek logs every call. The realism runner
already scores the capture with nDPI and fails it on a parser error. This check
adds what Zeek reads from the SIP dialogs:

* ``sip.log`` holds an INVITE row for every call, and the calls have distinct
  Call-IDs, so no call hides inside another one,
* every call has a row that carries a final status code, which means that Zeek
  parsed a reply to the dialog (the answer of the INVITE, or the 200 OK of the
  BYE), and
* ``weird.log`` holds no entry whose name starts with ``sip``.

Zeek logs the SIP dialogs only. The RTP streams have no Zeek log, so the RTP
packet interval has its own check, rtp_check.py.

Usage::

    zeek_check.py CAPTURE --calls N [--zeek PROGRAM]

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


def read_log(path: Path) -> list[dict]:
    """Return the rows of a JSON Zeek log, or an empty list when it is missing."""
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return []
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def _status(row: dict) -> int:
    try:
        return int(row.get("status_code") or 0)
    except (TypeError, ValueError):
        return 0


def check(logs: dict[str, list[dict]], calls: int) -> list[str]:
    """Return one line for each failed check, or an empty list when all pass."""
    errors: list[str] = []
    rows = logs.get("sip", [])
    weird = [
        w
        for w in logs.get("weird", [])
        if str(w.get("name", "")).lower().startswith("sip")
    ]

    for w in weird:
        errors.append(
            f"weird.log: {w.get('name')} on {w.get('id.orig_h')} to {w.get('id.resp_h')}"
        )

    invited = {
        r.get("call_id") for r in rows if str(r.get("method", "")).upper() == "INVITE"
    }
    invited.discard(None)
    if len(invited) != calls:
        errors.append(
            f"sip.log: INVITE rows for {len(invited)} different calls, expected {calls}"
        )

    by_call: dict[str, list[dict]] = {}
    for r in rows:
        by_call.setdefault(str(r.get("call_id")), []).append(r)
    for call_id in sorted(invited):
        if not any(_status(r) >= 200 for r in by_call.get(str(call_id), [])):
            errors.append(
                f"sip.log: call {call_id} has no row with a final status code"
            )
    return errors


def run_zeek(
    program: str, capture: Path, timeout_s: float = 300.0
) -> dict[str, list[dict]]:
    """Run Zeek on the capture and return the rows of the logs that the check reads."""
    with tempfile.TemporaryDirectory(prefix="fps-sip-zeek-") as tmp:
        cwd = Path(tmp)
        argv = [program, "-C", "-r", str(capture.resolve()), "LogAscii::use_json=T"]
        proc = subprocess.run(
            argv,
            cwd=cwd,
            capture_output=True,
            text=True,
            timeout=timeout_s,
            check=False,
        )
        if proc.returncode != 0:
            raise RuntimeError(
                f"zeek exited with {proc.returncode}: {proc.stderr.strip()}"
            )
        return {name: read_log(cwd / f"{name}.log") for name in ("sip", "weird")}


def main(argv: list[str] | None = None) -> int:
    """Run the check and print the result."""
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("capture", type=Path)
    p.add_argument("--calls", type=int, required=True)
    p.add_argument("--zeek", default=shutil.which("zeek") or DEFAULT_ZEEK)
    args = p.parse_args(argv)
    try:
        logs = run_zeek(args.zeek, args.capture)
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"zeek_check: {exc}", file=sys.stderr)
        return 2
    errors = check(logs, args.calls)
    print(
        f"zeek_check: sip.log {len(logs['sip'])} rows, weird.log {len(logs['weird'])} rows"
    )
    for row in logs["sip"][:4]:
        print(f"zeek_check: sip row {json.dumps(row, sort_keys=True)}")
    for e in errors:
        print(f"zeek_check: FAIL {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
