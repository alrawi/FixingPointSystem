# SPDX-License-Identifier: GPL-3.0-only
"""Command line of the capture tools."""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path
from typing import Any

from .decode import Packet, Skip, decode
from .pcapio import CaptureError, open_capture
from .sessions import DEFAULT_UDP_IDLE_S, build_document


def extract(
    path: Path,
    *,
    udp_idle_s: float = DEFAULT_UDP_IDLE_S,
    payloads: bool = False,
    interface: int | None = None,
) -> dict[str, Any]:
    """Read a capture file and return its session document."""
    reader, mapping = open_capture(path)
    try:
        packets: list[Packet] = []
        skipped: Counter[str] = Counter()
        frames = 0
        unordered = 0
        last_ts = 0
        for frame in reader.frames():
            if interface is not None and frame.interface != interface:
                continue
            frames += 1
            result = decode(frame)
            if isinstance(result, Skip):
                skipped[result.reason] += 1
                continue
            unordered += result.ts_ns < last_ts
            last_ts = max(last_ts, result.ts_ns)
            packets.append(result)
        if unordered:
            packets.sort(key=lambda p: p.ts_ns)  # stable, so equal times keep file order
        warnings = list(reader.warnings)
        if unordered:
            warnings.append(f"{unordered} packets had earlier timestamps than their predecessors")
        source = {
            "file": path.name,
            "format": reader.format,
            "frames": frames,
            "packets": len(packets),
            "skipped_frames": dict(sorted(skipped.items())),
            "warnings": warnings,
        }
        return build_document(
            packets,
            source=source,
            interfaces=reader.interfaces,
            udp_idle_s=udp_idle_s,
            payloads=payloads,
        )
    finally:
        if mapping is not None:
            mapping.close()


def main(argv: list[str] | None = None) -> int:
    """Run the command line and return the exit code."""
    parser = argparse.ArgumentParser(prog="python3 -m capture", description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("sessions", help="extract TCP sessions and UDP flows as JSON")
    p.add_argument("--pcap", type=Path, required=True, help="pcap or pcapng capture")
    p.add_argument("--out", type=Path, help="output file, default standard output")
    p.add_argument(
        "--udp-idle-timeout",
        type=float,
        default=DEFAULT_UDP_IDLE_S,
        metavar="SECONDS",
        help="a UDP flow idle for longer starts a new session (default %(default)s)",
    )
    p.add_argument("--payloads", action="store_true", help="include payload bytes as base64")
    p.add_argument("--interface", type=int, help="read only frames of this interface id")
    args = parser.parse_args(argv)
    if args.udp_idle_timeout <= 0:
        parser.error("--udp-idle-timeout must be positive")
    try:
        doc = extract(
            args.pcap,
            udp_idle_s=args.udp_idle_timeout,
            payloads=args.payloads,
            interface=args.interface,
        )
    except CaptureError as exc:
        print(f"capture: {exc}", file=sys.stderr)
        return 1
    text = json.dumps(doc, indent=2) + "\n"
    if args.out is None:
        sys.stdout.write(text)
        return 0
    try:
        args.out.write_text(text, encoding="utf-8")
    except OSError as exc:
        print(f"capture: cannot write {args.out}: {exc}", file=sys.stderr)
        return 1
    return 0
