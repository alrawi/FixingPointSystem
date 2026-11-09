#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Check the session structure of the ftp fixture: one control connection per
FTP session and exactly one data connection per transfer.

The acceptance of MOD-10 says that the session structure check reproduces one
control and one data connection per transfer. The realism check of that name
(RLM-08) does not exist yet, so this script reads the capture with the CAP-01
extractor ``python3 -m capture sessions --payloads`` and counts the connections
itself. For every control connection, which is a TCP session to the control
port, it reads the commands and the replies and requires that

* each LIST, NLST, MLSD, RETR, STOR, APPE, and STOU follows a 227 or 229 reply
  that announces a passive data port,
* exactly one other TCP session of the same client and server addresses starts
  while the control connection is open for each of those transfers, no more
  and no fewer, so a refused or repeated data connection fails the check,
* the data connection of the n-th transfer goes to the port that the n-th 227
  or 229 reply announced, and
* every session of the capture, control or data, completes its handshake,
  carries bytes, and ends with a FIN of both sides.

Usage::

    structure_check.py CAPTURE [--control-port 21] [--controls N] [--transfers M]

``--controls`` and ``--transfers`` give the number of control connections and
of transfers that the capture must hold. The script exits with 0 when every
check passes, 1 when a check fails, and 2 when the extractor cannot run.
"""

from __future__ import annotations

import argparse
import base64
import json
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

TRANSFER_COMMANDS = ("LIST", "NLST", "MLSD", "RETR", "STOR", "APPE", "STOU")
ACTIVE_COMMANDS = ("PORT", "EPRT")
ROOT = Path(__file__).resolve().parents[3]

_PASV = re.compile(rb"^227 .*\(\s*\d+,\s*\d+,\s*\d+,\s*\d+,\s*(\d+),\s*(\d+)\s*\)")
_EPSV = re.compile(rb"^229 .*\(\|\|\|(\d+)\|\)")


def extract_sessions(capture: Path, root: Path = ROOT) -> dict:
    """Run the CAP-01 extractor on the capture and return its JSON document."""
    proc = subprocess.run(
        [sys.executable, "-m", "capture", "sessions", "--pcap", str(capture), "--payloads"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"capture sessions exited with {proc.returncode}: {proc.stderr.strip()}")
    return json.loads(proc.stdout)


def stream(session: dict, direction: str) -> bytes:
    """Return the bytes that one direction of a TCP session carried."""
    return b"".join(
        base64.b64decode(t.get("data_b64", ""))
        for t in session.get("turns", [])
        if t["direction"] == direction
    )


def lines(data: bytes) -> list[bytes]:
    """Split a control stream into lines without the CRLF."""
    return [ln for ln in data.split(b"\r\n") if ln]


def announced_ports(replies: list[bytes]) -> list[int]:
    """Return the data ports of the 227 and 229 replies, in order."""
    ports = []
    for line in replies:
        if m := _PASV.match(line):
            ports.append(int(m.group(1)) * 256 + int(m.group(2)))
        elif m := _EPSV.match(line):
            ports.append(int(m.group(1)))
    return ports


def check(doc: dict, control_port: int = 21, controls: int | None = None,
          transfers: int | None = None) -> tuple[list[str], dict[str, int]]:
    """Return the failed checks and the counts of a sessions document."""
    errors: list[str] = []
    sessions = [s for s in doc["sessions"] if s["protocol"] == "tcp"]
    control = [s for s in sessions if s["server"]["port"] == control_port]
    data = [s for s in sessions if s["server"]["port"] != control_port]
    counts: Counter[str] = Counter(control=len(control), data=len(data))

    for s in sessions:
        what = f"session {s['id']} to {s['server']['ip']}:{s['server']['port']}"
        payload = s["client_to_server"]["payload_bytes"] + s["server_to_client"]["payload_bytes"]
        if s["handshake"]["state"] != "complete" or not payload:
            errors.append(f"{what}: refused or empty connection ({s['end_reason']})")
        elif s["end_reason"] != "fin":
            errors.append(f"{what}: ends with {s['end_reason']}, not FIN")

    owner: dict[int, int] = {}
    for c in control:
        cid = c["id"]
        commands = lines(stream(c, "client_to_server"))
        replies = lines(stream(c, "server_to_client"))
        verbs = [ln.split(b" ", 1)[0].upper().decode("ascii", "replace") for ln in commands]
        for v in verbs:
            if v in ACTIVE_COMMANDS:
                errors.append(f"control session {cid}: {v} is active mode, which this module lacks")
        transfer_cmds = [v for v in verbs if v in TRANSFER_COMMANDS]
        ports = announced_ports(replies)
        counts["transfers"] += len(transfer_cmds)
        for v in transfer_cmds:
            counts[v] += 1
        if len(ports) < len(transfer_cmds):
            errors.append(
                f"control session {cid}: {len(transfer_cmds)} transfers but only "
                f"{len(ports)} passive replies"
            )

        mine = [
            d for d in data
            if d["client"]["ip"] == c["client"]["ip"]
            and d["server"]["ip"] == c["server"]["ip"]
            and c["start_ns"] <= d["start_ns"] <= c["end_ns"]
        ]
        for d in mine:
            if d["id"] in owner:
                errors.append(f"data session {d['id']} lies in control sessions {owner[d['id']]} and {cid}")
            owner[d["id"]] = cid
        mine.sort(key=lambda d: d["start_ns"])
        if len(mine) != len(transfer_cmds):
            errors.append(
                f"control session {cid}: {len(transfer_cmds)} transfers but "
                f"{len(mine)} data connections"
            )
            continue
        for n, (d, port) in enumerate(zip(mine, ports), start=1):
            if d["server"]["port"] != port:
                errors.append(
                    f"control session {cid}: data connection {n} goes to port "
                    f"{d['server']['port']}, the reply announced {port}"
                )
            if d["start_ns"] < c["start_ns"]:
                errors.append(f"control session {cid}: data connection {n} starts first")

    for d in data:
        if d["id"] not in owner:
            errors.append(f"data session {d['id']} belongs to no control connection")
    if controls is not None and len(control) != controls:
        errors.append(f"{len(control)} control connections, expected {controls}")
    if transfers is not None and counts["transfers"] != transfers:
        errors.append(f"{counts['transfers']} transfers, expected {transfers}")
    if counts["data"] != counts["transfers"]:
        errors.append(f"{counts['data']} data connections for {counts['transfers']} transfers")
    return errors, dict(counts)


def main(argv: list[str] | None = None) -> int:
    """Run the check and print the result."""
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("capture", type=Path)
    p.add_argument("--control-port", type=int, default=21)
    p.add_argument("--controls", type=int)
    p.add_argument("--transfers", type=int)
    args = p.parse_args(argv)
    try:
        doc = extract_sessions(args.capture)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"structure_check: {exc}", file=sys.stderr)
        return 2
    errors, counts = check(doc, args.control_port, args.controls, args.transfers)
    per_verb = ", ".join(f"{v} {counts[v]}" for v in TRANSFER_COMMANDS if counts.get(v))
    print(
        f"structure_check: {counts['control']} control connections, "
        f"{counts['transfers']} transfers ({per_verb}), {counts['data']} data connections"
    )
    for e in errors:
        print(f"structure_check: FAIL {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
