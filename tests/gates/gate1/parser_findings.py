# SPDX-License-Identifier: GPL-3.0-only
"""Parser findings of a Gate 1 run, kept as files.

The script reads ``realism.json`` and ``gate1.pcapng`` of an output directory
and writes the parser findings of the run, so an artifact upload can carry them
when the parser error check fails.

    python3 -I parser_findings.py --out-dir OUT [--boundary-s 2.0]

It writes three files to OUT.

* ``parser-findings.json`` holds one entry per finding with the parser, the kind,
  the frame, the session, and the facts of the session that decide whether the
  capture boundary cut it.
* ``parser-findings.csv`` holds the same entries without the nested fields.
* ``parser-findings.pcapng`` holds every frame of the sessions that carry a
  finding, so the fault can be replayed in tshark or Zeek without the whole
  capture. It exists only when there is a finding with a session.

The JSON also lists ``reused_sessions``, the TCP sessions that hold more than one
connection, because two connections with one 5-tuple, which the kernel allows
once the old connection left TIME_WAIT on the server side, form one session of
the realism runner. A finding in such a session carries ``reused_tuple: true``.

A session counts as cut at the capture start when it has no SYN without ACK and
its first frame lies within ``--boundary-s`` seconds of the first frame of the
capture, and as cut at the capture stop when it carries no FIN or RST of either
side and its last frame lies within ``--boundary-s`` seconds of the last frame
of the capture. The script never fails the run: it exits with 0 whenever it
wrote the files, and with 2 when an input is unreadable.
"""

from __future__ import annotations

import argparse
import csv
import json
import struct
import sys
from collections.abc import Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from realism import capture as cap

BOUNDARY_S = 2.0
TCP_FIN = 0x01
TCP_SYN = 0x02
TCP_RST = 0x04
TCP_ACK = 0x10
CSV_FIELDS = (
    "parser",
    "kind",
    "name",
    "message",
    "frame",
    "timestamp",
    "offset_s",
    "session",
    "application",
    "transport",
    "client",
    "server",
    "session_packets",
    "session_syns",
    "session_fins",
    "session_rsts",
    "session_first_s",
    "session_last_s",
    "cut",
    "reused_tuple",
)


class FindingsError(Exception):
    """An input file is missing or unreadable."""


@dataclass
class SessionFacts:
    """What the packets of one session say about its boundaries."""

    packets: int = 0
    syns: int = 0
    fins: int = 0
    rsts: int = 0
    first: float = 0.0
    last: float = 0.0
    frames: list[int] = field(default_factory=list)
    # The initial sequence numbers of the SYN packets without ACK. A retransmitted
    # SYN repeats its number, so two numbers mean two connections.
    syn_seqs: set[int] = field(default_factory=set)

    @property
    def reused(self) -> bool:
        """True when the session holds more than one TCP connection."""
        return len(self.syn_seqs) > 1


def classify(
    facts: SessionFacts, transport: str, start: float, end: float, boundary_s: float
) -> str:
    """Return ``start``, ``stop``, ``both``, or ``none`` for the session facts."""
    if transport != "tcp":
        return "none"
    at_start = facts.syns == 0 and facts.first - start <= boundary_s
    at_stop = facts.fins == 0 and facts.rsts == 0 and end - facts.last <= boundary_s
    if at_start and at_stop:
        return "both"
    if at_start:
        return "start"
    return "stop" if at_stop else "none"


def endpoint_text(value: Any) -> str:
    """Format a ``{ip, port}`` entry of the realism report."""
    if isinstance(value, dict) and "ip" in value and "port" in value:
        return f"{value['ip']}:{value['port']}"
    return ""


def load_report(path: Path) -> dict[str, Any]:
    """Read the realism report and return its parser error check, or an empty check."""
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as e:
        raise FindingsError(f"{path} is missing") from e
    except (OSError, ValueError) as e:
        raise FindingsError(f"{path} is not readable JSON: {e}") from e
    for check in report.get("checks", []) if isinstance(report, dict) else []:
        if isinstance(check, dict) and check.get("check") == "parser_errors":
            return check
    return {}


def read_sessions(
    pcap: Path,
) -> tuple[
    dict[int, SessionFacts], dict[int, float], dict[int, list[cap.Frame]], float, float
]:
    """Read the capture and return the facts of each session by id.

    The session ids and the frame numbers follow the realism runner, which counts
    sessions in order of their first frame and frames from 1. The result also
    holds the timestamp of every frame, the frames of every session, and the
    timestamps of the first and the last frame of the capture.
    """
    try:
        _, frames = cap.read_frames(pcap)
        facts: dict[int, SessionFacts] = {}
        ids: dict[cap.SessionKey, int] = {}
        stamps: dict[int, float] = {}
        by_session: dict[int, list[cap.Frame]] = {}
        first = last = 0.0
        for number, frame in enumerate(frames, start=1):
            stamps[number] = frame.timestamp
            if number == 1:
                first = frame.timestamp
            first = min(first, frame.timestamp)
            last = max(last, frame.timestamp)
            segment = cap.decode_segment(frame)
            if segment is None:
                continue
            packet = segment.packet
            key = cap.SessionKey.of(
                packet.transport,
                packet.src_ip,
                packet.src_port,
                packet.dst_ip,
                packet.dst_port,
            )
            sid = ids.setdefault(key, len(ids) + 1)
            fact = facts.get(sid)
            if fact is None:
                fact = facts[sid] = SessionFacts(
                    first=packet.timestamp, last=packet.timestamp
                )
            fact.packets += 1
            fact.first = min(fact.first, packet.timestamp)
            fact.last = max(fact.last, packet.timestamp)
            fact.frames.append(number)
            by_session.setdefault(sid, []).append(frame)
            flags = packet.tcp_flags
            if flags & TCP_SYN and not flags & TCP_ACK:
                fact.syns += 1
                fact.syn_seqs.add(segment.seq)
            if flags & TCP_FIN:
                fact.fins += 1
            if flags & TCP_RST:
                fact.rsts += 1
    except cap.CaptureError as e:
        raise FindingsError(str(e)) from e
    return facts, stamps, by_session, first, last


def build(
    check: dict[str, Any],
    facts: dict[int, SessionFacts],
    stamps: dict[int, float],
    start: float,
    end: float,
    boundary_s: float,
) -> list[dict[str, Any]]:
    """Return one entry per finding of the parser error check."""
    entries: list[dict[str, Any]] = []
    for raw in check.get("errors", []):
        if not isinstance(raw, dict):
            continue
        entry: dict[str, Any] = {
            key: raw.get(key)
            for key in (
                "parser",
                "kind",
                "name",
                "message",
                "frame",
                "timestamp",
                "session",
                "application",
                "transport",
                "client",
                "server",
            )
        }
        stamp = entry["timestamp"]
        if stamp is None and isinstance(entry["frame"], int):
            stamp = stamps.get(entry["frame"])
        entry["offset_s"] = None if stamp is None else round(float(stamp) - start, 6)
        fact = (
            facts.get(entry["session"]) if isinstance(entry["session"], int) else None
        )
        if fact is not None:
            entry["session_facts"] = {
                "packets": fact.packets,
                "syns": fact.syns,
                "connections": max(len(fact.syn_seqs), 1),
                "fins": fact.fins,
                "rsts": fact.rsts,
                "first_s": round(fact.first - start, 6),
                "last_s": round(fact.last - start, 6),
                "frames": fact.frames[:200],
            }
            entry["cut"] = classify(
                fact, str(entry["transport"] or ""), start, end, boundary_s
            )
            entry["reused_tuple"] = fact.reused
        else:
            entry["session_facts"] = None
            entry["cut"] = "unknown"
            entry["reused_tuple"] = False
        entries.append(entry)
    return entries


def _block(block_type: int, body: bytes) -> bytes:
    body += b"\x00" * (-len(body) % 4)
    total = len(body) + 12
    return struct.pack("<II", block_type, total) + body + struct.pack("<I", total)


def pcapng_bytes(link: int, frames: Sequence[tuple[float, bytes]]) -> bytes:
    """Return a pcapng file of one interface with nanosecond timestamps."""
    out = bytearray(_block(0x0A0D0D0A, struct.pack("<IHHq", 0x1A2B3C4D, 1, 0, -1)))
    options = struct.pack("<HHBxxx", 9, 1, 9) + struct.pack("<HH", 0, 0)
    out += _block(1, struct.pack("<HHI", link, 0, 262144) + options)
    for stamp, data in frames:
        ticks = round(stamp * 1e9)
        header = struct.pack(
            "<IIIII", 0, ticks >> 32, ticks & 0xFFFFFFFF, len(data), len(data)
        )
        out += _block(6, header + data)
    return bytes(out)


def write_capture(path: Path, frames: Sequence[cap.Frame]) -> None:
    """Write frames to a pcapng file of the link type of the first frame."""
    ordered = sorted(frames, key=lambda f: f.timestamp)
    path.write_bytes(
        pcapng_bytes(ordered[0].link_type, [(f.timestamp, f.data) for f in ordered])
    )


def write_files(
    out: Path,
    check: dict[str, Any],
    entries: list[dict[str, Any]],
    by_session: dict[int, list[cap.Frame]],
    facts: dict[int, SessionFacts],
    start: float,
    end: float,
    boundary_s: float,
) -> None:
    """Write the JSON, the CSV, and the capture of the findings to the directory."""
    reused = [
        {
            "session": sid,
            "connections": len(fact.syn_seqs),
            "first_s": round(fact.first - start, 6),
            "last_s": round(fact.last - start, 6),
        }
        for sid, fact in sorted(facts.items())
        if fact.reused
    ]
    doc = {
        "parsers": check.get("parsers", []),
        "errors_total": check.get("errors_total", 0),
        "listed": len(entries),
        "boundary_s": boundary_s,
        "capture_start": start,
        "capture_end": end,
        "reused_sessions": reused,
        "findings": entries,
    }
    (out / "parser-findings.json").write_text(
        json.dumps(doc, indent=2) + "\n", encoding="utf-8"
    )
    with (out / "parser-findings.csv").open("w", encoding="utf-8", newline="") as fh:
        writer = csv.writer(fh)
        writer.writerow(CSV_FIELDS)
        for e in entries:
            f = e["session_facts"] or {}
            writer.writerow(
                [
                    e["parser"],
                    e["kind"],
                    e["name"],
                    e["message"],
                    e["frame"],
                    e["timestamp"],
                    e["offset_s"],
                    e["session"],
                    e["application"],
                    e["transport"],
                    endpoint_text(e["client"]),
                    endpoint_text(e["server"]),
                    f.get("packets", ""),
                    f.get("syns", ""),
                    f.get("fins", ""),
                    f.get("rsts", ""),
                    f.get("first_s", ""),
                    f.get("last_s", ""),
                    e["cut"],
                    e["reused_tuple"],
                ]
            )
    pcap_path = out / "parser-findings.pcapng"
    pcap_path.unlink(missing_ok=True)
    wanted = sorted({e["session"] for e in entries if isinstance(e["session"], int)})
    frames = [f for sid in wanted for f in by_session.get(sid, [])]
    if frames:
        write_capture(pcap_path, frames)


def run(out: Path, boundary_s: float) -> list[dict[str, Any]]:
    """Write the findings files of an output directory and return the entries."""
    check = load_report(out / "realism.json")
    pcap = out / "gate1.pcapng"
    if not pcap.is_file():
        raise FindingsError(f"{pcap} is missing")
    facts, stamps, by_session, start, end = read_sessions(pcap)
    entries = build(check, facts, stamps, start, end, boundary_s)
    write_files(out, check, entries, by_session, facts, start, end, boundary_s)
    return entries


def main(argv: list[str] | None = None) -> int:
    """Run the command line interface."""
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--out-dir", type=Path, required=True)
    p.add_argument("--boundary-s", type=float, default=BOUNDARY_S)
    args = p.parse_args(argv)
    try:
        entries = run(args.out_dir, args.boundary_s)
    except FindingsError as e:
        print(f"parser_findings.py: {e}", file=sys.stderr)
        return 2
    for found in entries:
        print(
            f"parser_findings.py: {found['parser']} {found['kind']} {found['name']} "
            f"frame {found['frame']} session {found['session']} cut {found['cut']}",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
