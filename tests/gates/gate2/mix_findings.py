# SPDX-License-Identifier: GPL-3.0-only
"""Parser findings of the Gate 2 mix stage, printed in the job log and kept as a file.

The script reads ``realism.json`` and ``mix.pcapng`` of the mix directory of an
output directory. It prints one line for each parser finding, so the job log names
every finding without the run artifacts, and writes ``parser-findings.json`` to the
mix directory.

    python3 -I mix_findings.py --out-dir OUT [--boundary-s 2.0]

A line says where the finding lies in the capture and what the packets of its
session say about the two causes that a finding without a protocol fault has had so
far. The agents pick source ports at random, so two connections can use one 5-tuple
and form one session of the realism runner (``reused_tuple``). The capture can also
start or stop inside a session (``cut``). A session counts as cut at the capture
start when it has no SYN without ACK and its first frame lies within ``--boundary-s``
seconds of the first frame of the capture, and as cut at the capture stop when it
carries no FIN or RST and its last frame lies within ``--boundary-s`` seconds of the
last frame. The file also lists the Zeek weirds and the tshark messages that the
findings share, with their counts.

The script never decides the stage. It exits with 0 whenever it printed the findings,
and with 2 when an input is unreadable.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
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
FINDING_KEYS = (
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


class FindingsError(Exception):
    """An input file is missing or unreadable."""


@dataclass
class SessionFacts:
    """What the packets of one session say about its boundaries and its connections."""

    packets: int = 0
    syns: int = 0
    fins: int = 0
    rsts: int = 0
    first: float = 0.0
    last: float = 0.0
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


def load_check(path: Path) -> dict[str, Any]:
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


def read_sessions(pcap: Path) -> tuple[dict[int, SessionFacts], float, float]:
    """Read the capture and return the facts of each session by id and the capture times.

    The session ids follow the realism runner, which counts sessions in the order of
    their first frame from 1.
    """
    try:
        _, frames = cap.read_frames(pcap)
        facts: dict[int, SessionFacts] = {}
        ids: dict[cap.SessionKey, int] = {}
        first = last = 0.0
        for number, frame in enumerate(frames, start=1):
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
    return facts, first, last


def endpoint_text(value: Any) -> str:
    """Format a ``{ip, port}`` entry of the realism report."""
    if isinstance(value, dict) and "ip" in value and "port" in value:
        return f"{value['ip']}:{value['port']}"
    return "?"


def build(
    check: dict[str, Any],
    facts: dict[int, SessionFacts],
    start: float,
    end: float,
    boundary_s: float,
) -> list[dict[str, Any]]:
    """Return one entry per finding that the realism report lists."""
    entries: list[dict[str, Any]] = []
    for raw in check.get("errors", []):
        if not isinstance(raw, dict):
            continue
        entry: dict[str, Any] = {key: raw.get(key) for key in FINDING_KEYS}
        stamp = entry["timestamp"]
        valid = isinstance(stamp, int | float) and not isinstance(stamp, bool)
        entry["offset_s"] = round(float(stamp) - start, 6) if valid else None
        sid = entry["session"]
        fact = facts.get(sid) if isinstance(sid, int) else None
        if fact is None:
            entry["session_facts"] = None
            entry["cut"] = "unknown"
            entry["reused_tuple"] = False
        else:
            entry["session_facts"] = {
                "packets": fact.packets,
                "syns": fact.syns,
                "connections": max(len(fact.syn_seqs), 1),
                "fins": fact.fins,
                "rsts": fact.rsts,
                "first_s": round(fact.first - start, 6),
                "last_s": round(fact.last - start, 6),
            }
            entry["cut"] = classify(
                fact, str(entry["transport"] or ""), start, end, boundary_s
            )
            entry["reused_tuple"] = fact.reused
        entries.append(entry)
    return entries


def describe(entry: dict[str, Any], duration: float) -> str:
    """Describe one finding on one line for the job log."""
    text = f"{entry['parser']} {entry['kind']} {entry['name']}"
    if entry["message"]:
        text += f" ({entry['message']})"
    where = []
    if entry["frame"] is not None:
        where.append(f"frame {entry['frame']}")
    if entry["offset_s"] is not None:
        where.append(f"at +{entry['offset_s']:.3f} s of {duration:.3f} s")
    text += ", " + ", ".join(where) if where else ""
    facts = entry["session_facts"]
    if facts is None:
        return text + ", no session of the capture"
    text += (
        f", session {entry['session']} of {entry['application'] or 'no application'}"
        f" ({entry['transport']} {endpoint_text(entry['client'])} ->"
        f" {endpoint_text(entry['server'])}, {facts['packets']} packets,"
        f" {facts['connections']} connection(s), {facts['fins']} FIN, {facts['rsts']} RST,"
        f" +{facts['first_s']:.3f} s to +{facts['last_s']:.3f} s)"
    )
    return (
        text
        + f", cut {entry['cut']}, reused_tuple {str(entry['reused_tuple']).lower()}"
    )


def run(out: Path, boundary_s: float) -> list[dict[str, Any]]:
    """Write the findings file of an output directory and return the entries."""
    mix = out / "mix"
    check = load_check(mix / "realism.json")
    pcap = mix / "mix.pcapng"
    if not pcap.is_file():
        raise FindingsError(f"{pcap} is missing")
    facts, start, end = read_sessions(pcap)
    entries = build(check, facts, start, end, boundary_s)
    doc = {
        "parsers": check.get("parsers", []),
        "errors_total": check.get("errors_total", 0),
        "listed": len(entries),
        "boundary_s": boundary_s,
        "capture_start": start,
        "capture_end": end,
        "capture_duration_s": round(end - start, 6),
        "by_name": dict(Counter(f"{e['parser']} {e['name']}" for e in entries)),
        "by_application": dict(Counter(str(e["application"]) for e in entries)),
        "reused_sessions": [sid for sid, fact in sorted(facts.items()) if fact.reused],
        "findings": entries,
    }
    (mix / "parser-findings.json").write_text(
        json.dumps(doc, indent=2) + "\n", encoding="utf-8"
    )
    return entries


def main(argv: Sequence[str] | None = None) -> int:
    """Print the parser findings of the mix stage and return the exit code."""
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else "")
    p.add_argument("--out-dir", type=Path, required=True)
    p.add_argument("--boundary-s", type=float, default=BOUNDARY_S)
    args = p.parse_args(argv)
    try:
        entries = run(args.out_dir, args.boundary_s)
        report = json.loads(
            (args.out_dir / "mix" / "parser-findings.json").read_text(encoding="utf-8")
        )
    except FindingsError as e:
        print(f"mix_findings.py: {e}", file=sys.stderr)
        return 2
    duration = float(report["capture_duration_s"])
    total = int(report["errors_total"])
    print(
        f"mix_findings.py: {total} parser findings in a capture of {duration:.3f} s,"
        f" {len(entries)} listed",
        file=sys.stderr,
    )
    for number, entry in enumerate(entries, start=1):
        print(
            f"mix_findings.py: {number}. {describe(entry, duration)}", file=sys.stderr
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
