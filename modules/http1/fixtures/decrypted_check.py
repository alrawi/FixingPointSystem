#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Measures the HTTP inside the TLS sessions of the http1 fixture.

nDPI cannot read inside TLS, so on the capture itself it classifies the https
sessions as TLS. This script decrypts the capture with tshark and the keys that
the capture embeds (or a key log from ``--keylog``), writes the plaintext of
every TLS session as a capture of plain TCP sessions with the same addresses and
ports, and scores that decrypted view with the realism runner: nDPI must
classify at least ``--min-ratio`` of the sessions as HTTP. It also counts the
HTTP requests and responses that tshark parses inside the TLS sessions of the
original capture.

    decrypted_check.py CAPTURE [--keylog FILE] [--work DIR] [--report FILE]
                       [--min-ratio 0.99] [--tshark PROGRAM]

The script needs python3 3.10 or newer, tshark 4.2 or newer, and the realism
runner of the repository (``python3 -m realism``) with ndpiReader. It exits with
0 when every TLS session decrypts, carries HTTP, and the nDPI ratio is reached,
with 1 when a check fails, and with 2 when a tool cannot run.
"""

from __future__ import annotations

import argparse
import json
import re
import struct
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
MSS = 1400
CLIENT_MAC = bytes.fromhex("020000000001")
SERVER_MAC = bytes.fromhex("020000000002")


class ToolError(Exception):
    """A tool did not run or printed output that this script cannot read."""


def run(argv: list[str]) -> str:
    """Run a program and return its standard output."""
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, check=False)
    except OSError as exc:
        raise ToolError(f"{argv[0]}: {exc.strerror}") from exc
    if proc.returncode != 0:
        raise ToolError(f"{argv[0]} exited with {proc.returncode}: {proc.stderr.strip()[-300:]}")
    return proc.stdout


def tshark_args(tshark: str, capture: Path, keylog: Path | None) -> list[str]:
    """Return the start of every tshark command line."""
    argv = [tshark, "-r", str(capture)]
    if keylog is not None:
        argv += ["-o", f"tls.keylog_file:{keylog}"]
    return argv


class Session:
    """One TLS session of the capture."""

    def __init__(self, stream: int, client: tuple[str, int], server: tuple[str, int]) -> None:
        self.stream = stream
        self.client = client
        self.server = server
        # The plaintext chunks in order, as (from_client, bytes).
        self.chunks: list[tuple[bool, bytes]] = []
        self.requests = 0
        self.responses = 0


def tls_sessions(base: list[str]) -> dict[int, Session]:
    """Return the sessions whose client sent a ClientHello, by TCP stream index."""
    fields = ["tcp.stream", "ip.src", "tcp.srcport", "ip.dst", "tcp.dstport"]
    argv = base + ["-Y", "tls.handshake.type == 1", "-T", "fields"]
    for field in fields:
        argv += ["-e", field]
    sessions: dict[int, Session] = {}
    for line in run(argv).splitlines():
        stream, src, sport, dst, dport = line.split("\t")
        sessions.setdefault(int(stream), Session(int(stream), (src, int(sport)), (dst, int(dport))))
    return sessions


def follow(base: list[str], sessions: dict[int, Session]) -> None:
    """Fill the plaintext chunks of every session from tshark's TLS follow."""
    if not sessions:
        return
    argv = base + ["-q"]
    for stream in sessions:
        argv += ["-z", f"follow,tls,raw,{stream}"]
    text = run(argv)
    current: Session | None = None
    for line in text.splitlines():
        header = re.fullmatch(r"Filter: tcp\.stream eq (\d+)", line)
        if header:
            current = sessions.get(int(header.group(1)))
            continue
        if current is None or not re.fullmatch(r"\t?[0-9a-f]+", line):
            continue
        # Node 0 is the server and node 1 is the client, which is indented.
        current.chunks.append((line.startswith("\t"), bytes.fromhex(line.strip())))


def count_http(base: list[str], sessions: dict[int, Session]) -> None:
    """Count the HTTP requests and responses that tshark parses in each session."""
    argv = base + ["-Y", "http", "-T", "fields", "-e", "tcp.stream", "-e", "http.request.method",
                   "-e", "http.response.code"]
    for line in run(argv).splitlines():
        parts = line.split("\t")
        session = sessions.get(int(parts[0]))
        if session is None:
            continue
        if len(parts) > 1 and parts[1]:
            session.requests += 1
        if len(parts) > 2 and parts[2]:
            session.responses += 1


def checksum(data: bytes) -> int:
    """Return the Internet checksum of the data."""
    if len(data) % 2:
        data += b"\0"
    total = sum(struct.unpack(f"!{len(data) // 2}H", data))
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return ~total & 0xFFFF


def ip_bytes(text: str) -> bytes:
    return bytes(int(part) for part in text.split("."))


def frame(src: tuple[str, int], dst: tuple[str, int], seq: int, ack: int, flags: int,
          payload: bytes, from_client: bool, ident: int) -> bytes:
    """Build an Ethernet, IPv4, and TCP frame with correct checksums."""
    tcp = struct.pack("!HHIIBBHHH", src[1], dst[1], seq, ack, 5 << 4, flags, 65535, 0, 0) + payload
    pseudo = ip_bytes(src[0]) + ip_bytes(dst[0]) + struct.pack("!BBH", 0, 6, len(tcp))
    tcp = tcp[:16] + struct.pack("!H", checksum(pseudo + tcp)) + tcp[18:]
    header = struct.pack("!BBHHHBBH", 0x45, 0, 20 + len(tcp), ident & 0xFFFF, 0x4000, 64, 6, 0)
    header += ip_bytes(src[0]) + ip_bytes(dst[0])
    header = header[:10] + struct.pack("!H", checksum(header)) + header[12:]
    macs = (SERVER_MAC + CLIENT_MAC) if from_client else (CLIENT_MAC + SERVER_MAC)
    return macs + b"\x08\x00" + header + tcp


def session_frames(session: Session, ident: int) -> list[bytes]:
    """Return the frames of one plain TCP session that carries the plaintext."""
    client, server = session.client, session.server
    syn, ack_flag, psh, fin = 0x02, 0x10, 0x08, 0x01
    c_seq, s_seq = 1000, 5000
    out = [
        frame(client, server, c_seq, 0, syn, b"", True, ident),
        frame(server, client, s_seq, c_seq + 1, syn | ack_flag, b"", False, ident + 1),
        frame(client, server, c_seq + 1, s_seq + 1, ack_flag, b"", True, ident + 2),
    ]
    c_seq, s_seq = c_seq + 1, s_seq + 1
    ident += 3
    for from_client, data in session.chunks:
        for start in range(0, len(data), MSS):
            piece = data[start:start + MSS]
            if from_client:
                out.append(frame(client, server, c_seq, s_seq, ack_flag | psh, piece, True, ident))
                c_seq += len(piece)
            else:
                out.append(frame(server, client, s_seq, c_seq, ack_flag | psh, piece, False, ident))
                s_seq += len(piece)
            ident += 1
    out.append(frame(client, server, c_seq, s_seq, fin | ack_flag, b"", True, ident))
    out.append(frame(server, client, s_seq, c_seq + 1, fin | ack_flag, b"", False, ident + 1))
    return out


def write_view(path: Path, sessions: list[Session]) -> None:
    """Write the plaintext of the sessions as a classic pcap capture."""
    with path.open("wb") as fh:
        fh.write(struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1))
        tick = 0
        ident = 1
        for session in sessions:
            frames = session_frames(session, ident)
            ident += len(frames)
            for data in frames:
                tick += 1000
                fh.write(struct.pack("<IIII", 1_700_000_000 + tick // 1_000_000, tick % 1_000_000,
                                     len(data), len(data)))
                fh.write(data)


def score_view(view: Path, ports: list[int], work: Path, min_ratio: float) -> dict:
    """Score the decrypted view with the realism runner and return its conformance entry."""
    labels = work / "decrypted.labels.yaml"
    lines = [
        "labels: fps-realism-labels", "version: 1", f"threshold: {min_ratio}", "sessions:",
    ]
    for port in ports:
        lines += ["  - application: https_decrypted",
                  f"    match: {{transport: tcp, server_port: {port}}}"]
    lines += ["applications:", "  https_decrypted: {ndpi: [HTTP]}"]
    labels.write_text("\n".join(lines) + "\n", encoding="utf-8")
    report = work / "decrypted.realism.json"
    proc = subprocess.run(
        [sys.executable, "-m", "realism", "score", "--pcap", str(view), "--labels", str(labels),
         "--parsers", "tshark", "--output", str(report)],
        cwd=ROOT, capture_output=True, text=True, check=False,
    )
    if proc.returncode == 2 or not report.is_file():
        raise ToolError(f"the realism runner could not score the view: {proc.stderr.strip()[-300:]}")
    data = json.loads(report.read_text(encoding="utf-8"))
    entry = next(c for c in data["checks"] if c["check"] == "conformance")
    entry["parser_errors"] = next(
        (c["errors_total"] for c in data["checks"] if c["check"] == "parser_errors"), None
    )
    return entry


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--keylog", type=Path, help="key log in the NSS format, when the capture embeds none")
    parser.add_argument("--work", type=Path, help="directory that keeps the decrypted view")
    parser.add_argument("--report", type=Path, help="file for the JSON report")
    parser.add_argument("--min-ratio", type=float, default=0.99)
    parser.add_argument("--tshark", default="tshark")
    args = parser.parse_args(argv)
    try:
        with tempfile.TemporaryDirectory(prefix="fps-decrypted-") as tmp:
            work = args.work or Path(tmp)
            work.mkdir(parents=True, exist_ok=True)
            base = tshark_args(args.tshark, args.capture, args.keylog)
            sessions = tls_sessions(base)
            if not sessions:
                raise ToolError("the capture has no TLS session")
            follow(base, sessions)
            count_http(base, sessions)
            decrypted = [s for s in sessions.values() if s.chunks]
            with_http = [s for s in decrypted if s.requests and s.requests == s.responses]
            ports = sorted({s.server[1] for s in sessions.values()})
            view = work / "http1.decrypted.pcap"
            write_view(view, decrypted)
            entry = score_view(view, ports, work, args.min_ratio)
    except ToolError as exc:
        print(f"decrypted_check: {exc}", file=sys.stderr)
        return 2
    application = entry["applications"][0] if entry["applications"] else {}
    total = len(sessions)
    result = {
        "capture": args.capture.name,
        "tls_sessions": total,
        "decrypted_sessions": len(decrypted),
        "sessions_with_http": len(with_http),
        "http_requests": sum(s.requests for s in sessions.values()),
        "http_responses": sum(s.responses for s in sessions.values()),
        "server_ports": dict(Counter(s.server[1] for s in sessions.values())),
        "decrypted_view": {
            "classifier": f"{entry['classifier']} {entry['classifier_version']}",
            "sessions": application.get("sessions", 0),
            "http": application.get("conforming", 0),
            "ratio": application.get("ratio", 0.0),
            "observed": application.get("observed", {}),
            "tshark_parser_errors": entry.get("parser_errors"),
        },
        "min_ratio": args.min_ratio,
    }
    result["passed"] = (
        len(decrypted) == total
        and len(with_http) == total
        and result["decrypted_view"]["sessions"] == total
        and result["decrypted_view"]["ratio"] >= args.min_ratio
    )
    if args.report:
        args.report.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    view_info = result["decrypted_view"]
    print(
        f"decrypted_check: {len(decrypted)} of {total} TLS sessions decrypt, "
        f"tshark parses {result['http_requests']} requests and {result['http_responses']} "
        f"responses in {len(with_http)} of them, {view_info['classifier']} classifies "
        f"{view_info['http']} of {view_info['sessions']} decrypted sessions as HTTP "
        f"(ratio {view_info['ratio']}, needs {args.min_ratio}): "
        f"{'passed' if result['passed'] else 'FAILED'}",
        file=sys.stderr,
    )
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
