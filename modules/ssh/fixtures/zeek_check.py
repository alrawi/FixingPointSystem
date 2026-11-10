#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Check that Zeek parses the sessions of the ssh fixture.

The acceptance of MOD-11 asks that the server and the client complete the key
exchange in every fixture session. The realism runner already scores the
capture with nDPI and fails it on a parser error. This check adds what Zeek
reads from the handshake:

* ``ssh.log`` holds one row for every session, and every row names the client
  and the server banner, the algorithms of the key exchange, the cipher, and
  the host key algorithm that the module negotiates,
* every row carries the same host key fingerprint, because the module derives
  the key of the server from one seed, and
* ``weird.log`` holds no entry whose name starts with ``ssh``.

Zeek stops parsing a session once the encryption starts, so the check does not
read the authentication result.

Usage::

    zeek_check.py CAPTURE --sessions N [--zeek PROGRAM]

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
KEX_ALG = "curve25519-sha256"
CIPHER_ALG = "chacha20-poly1305@openssh.com"
HOST_KEY_ALG = "ssh-ed25519"


def read_log(path: Path) -> list[dict]:
    """Return the rows of a JSON Zeek log, or an empty list when it is missing."""
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return []
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def check(logs: dict[str, list[dict]], sessions: int) -> list[str]:
    """Return one line for each failed check, or an empty list when all pass."""
    errors: list[str] = []
    rows = logs.get("ssh", [])
    weird = [w for w in logs.get("weird", []) if str(w.get("name", "")).lower().startswith("ssh")]

    for w in weird:
        errors.append(f"weird.log: {w.get('name')} on {w.get('id.orig_h')} to {w.get('id.resp_h')}")
    if len(rows) != sessions:
        errors.append(f"ssh.log: {len(rows)} rows, expected {sessions}")
    for i, row in enumerate(rows):
        what = f"ssh.log row {i + 1}"
        for key in ("client", "server"):
            if not str(row.get(key, "")).strip():
                errors.append(f"{what} lacks {key}")
        for key, want in (
            ("kex_alg", KEX_ALG),
            ("cipher_alg", CIPHER_ALG),
            ("host_key_alg", HOST_KEY_ALG),
        ):
            if row.get(key) != want:
                errors.append(f"{what} has {key} {row.get(key)!r}, expected {want!r}")
    keys = {row.get("host_key") for row in rows if row.get("host_key")}
    if len(keys) > 1:
        errors.append(f"ssh.log: {len(keys)} different host keys, expected one")
    return errors


def run_zeek(program: str, capture: Path, timeout_s: float = 300.0) -> dict[str, list[dict]]:
    """Run Zeek on the capture and return the rows of the logs that the check reads."""
    with tempfile.TemporaryDirectory(prefix="fps-ssh-zeek-") as tmp:
        cwd = Path(tmp)
        argv = [program, "-C", "-r", str(capture.resolve()), "LogAscii::use_json=T"]
        proc = subprocess.run(
            argv, cwd=cwd, capture_output=True, text=True, timeout=timeout_s, check=False
        )
        if proc.returncode != 0:
            raise RuntimeError(f"zeek exited with {proc.returncode}: {proc.stderr.strip()}")
        return {name: read_log(cwd / f"{name}.log") for name in ("ssh", "weird")}


def main(argv: list[str] | None = None) -> int:
    """Run the check and print the result."""
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("capture", type=Path)
    p.add_argument("--sessions", type=int, required=True)
    p.add_argument("--zeek", default=shutil.which("zeek") or DEFAULT_ZEEK)
    args = p.parse_args(argv)
    try:
        logs = run_zeek(args.zeek, args.capture)
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"zeek_check: {exc}", file=sys.stderr)
        return 2
    errors = check(logs, args.sessions)
    print(f"zeek_check: ssh.log {len(logs['ssh'])} rows, weird.log {len(logs['weird'])} rows")
    print(f"zeek_check: first ssh row {json.dumps(next(iter(logs['ssh']), None), sort_keys=True)}")
    for e in errors:
        print(f"zeek_check: FAIL {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
