#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Measures the SMTP inside the STARTTLS sessions of the smtp fixture.

The sessions of the scenario ``starttls_submission`` start in plain text, move to
TLS after ``STARTTLS``, and carry the authentication and the message inside TLS.
nDPI reads only the plaintext start of such a session, and Zeek has no way to
decrypt TLS (it takes no key log), so neither tool can see the message on the
capture. This script decrypts the capture with tshark and the keys that the
capture embeds (or a key log from ``--keylog``) and then measures three things.

1. tshark parses the SMTP dialogue inside every TLS session of the capture itself
   (it decodes the TLS port as SMTP), so each session shows its ``EHLO`` after
   the upgrade, ``AUTH``, ``MAIL``, ``RCPT``, ``DATA``, an RFC 5322 message, and
   ``QUIT``, with the replies 235, 354, and 221.
2. nDPI classifies at least ``--min-ratio`` of the sessions of a decrypted view as
   SMTP. The view is a capture of plain TCP sessions with the addresses of the
   original sessions, on port ``--view-port`` (587, the port of the scenario),
   whose payload is the plaintext of each session: what both sides sent before
   the upgrade, without the ``STARTTLS`` command and its 220 reply, followed by
   what they sent inside TLS. The two lines go because Zeek and nDPI stop reading
   a session at ``STARTTLS``, and the bytes after it would be TLS in a real
   capture. The view is synthetic, built from the output of tshark.
3. With ``--zeek``, Zeek reads the view, and ``smtp.log`` must hold one message
   per session with the sender, the recipients, the header fields, and the file
   of the attachment, ``files.log`` must hold the attachment of each, and
   ``weird.log`` no smtp entry. These are the checks of ``zeek_check.py``.

    decrypted_check.py CAPTURE --sessions N [--keylog FILE] [--work DIR]
                       [--report FILE] [--zeek PROGRAM] [--min-ratio 0.99]
                       [--view-port 587] [--tshark PROGRAM]

``--sessions`` is the number of STARTTLS sessions that the capture holds. The
script needs python3 3.10 or newer, tshark 4.2 or newer, and the realism runner
of the repository (``python3 -m realism``) with ndpiReader. It exits with 0 when
every check passes, with 1 when a check fails, and with 2 when a tool cannot run.
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
sys.path.insert(0, str(Path(__file__).resolve().parent))

import zeek_check  # noqa: E402  (the path above makes it importable under python3 -I)

MSS = 1400
CLIENT_MAC = bytes.fromhex("020000000001")
SERVER_MAC = bytes.fromhex("020000000002")
REQUIRED_COMMANDS = ("EHLO", "AUTH", "MAIL", "RCPT", "DATA", "QUIT")
REQUIRED_REPLIES = ("235", "354", "221")


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
    """One STARTTLS session of the capture."""

    def __init__(self, stream: int, hello: int, client: tuple[str, int], server: tuple[str, int]) -> None:
        self.stream = stream
        self.hello = hello
        self.client = client
        self.server = server
        # The plaintext before the ClientHello and the plaintext inside TLS, each
        # in order, as (from_client, bytes).
        self.before: list[tuple[bool, bytes]] = []
        self.inside: list[tuple[bool, bytes]] = []
        self.commands: Counter[str] = Counter()
        self.replies: Counter[str] = Counter()
        self.messages = 0

    def starttls_offered(self) -> bool:
        """True when the first reply offered STARTTLS and the client asked for it."""
        server = b"".join(b for from_client, b in self.before if not from_client)
        client = b"".join(b for from_client, b in self.before if from_client)
        return b"250-STARTTLS" in server and b"STARTTLS\r\n" in client and b"\r\n220 " in server

    def upgraded_in_dialogue(self) -> list[tuple[bool, bytes]]:
        """The plaintext before the upgrade without the STARTTLS command and its reply."""
        out: list[tuple[bool, bytes]] = []
        skip_reply = False
        for from_client, data in self.before:
            if from_client and data.strip().upper() == b"STARTTLS":
                skip_reply = True
                continue
            if skip_reply and not from_client:
                skip_reply = False
                continue
            out.append((from_client, data))
        return out


def tls_sessions(base: list[str]) -> dict[int, Session]:
    """Return the sessions whose client sent a ClientHello, by TCP stream index."""
    fields = ["tcp.stream", "frame.number", "ip.src", "tcp.srcport", "ip.dst", "tcp.dstport"]
    argv = base + ["-Y", "tls.handshake.type == 1", "-T", "fields"]
    for field in fields:
        argv += ["-e", field]
    sessions: dict[int, Session] = {}
    for line in run(argv).splitlines():
        stream, frame, src, sport, dst, dport = line.split("\t")
        sessions.setdefault(
            int(stream), Session(int(stream), int(frame), (src, int(sport)), (dst, int(dport)))
        )
    return sessions


def plaintext_before(base: list[str], sessions: dict[int, Session]) -> None:
    """Fill what each side sent before the ClientHello, which is the plaintext of the upgrade."""
    argv = base + ["-Y", "tcp.len > 0", "-T", "fields"]
    for field in ("frame.number", "tcp.stream", "ip.src", "tcp.srcport", "tcp.seq", "tcp.payload"):
        argv += ["-e", field]
    seen: set[tuple[int, bool, int]] = set()
    for line in run(argv).splitlines():
        frame, stream, src, sport, seq, payload = line.split("\t")
        session = sessions.get(int(stream))
        if session is None or int(frame) >= session.hello or not payload:
            continue
        from_client = (src, int(sport)) == session.client
        key = (session.stream, from_client, int(seq))
        if key in seen:
            continue
        seen.add(key)
        session.before.append((from_client, bytes.fromhex(payload.replace(":", ""))))


def follow(base: list[str], sessions: dict[int, Session]) -> None:
    """Fill the plaintext inside TLS of every session from tshark's TLS follow."""
    if not sessions:
        return
    argv = base + ["-q"]
    for stream in sessions:
        argv += ["-z", f"follow,tls,raw,{stream}"]
    current: Session | None = None
    # Which of the two nodes is the client, which the header of each stream says:
    # node 0 is the endpoint that sent the first bytes inside TLS, which is the
    # server in TLS 1.3 for some dialogues and the client in others.
    client_node = 1
    for line in run(argv).splitlines():
        header = re.fullmatch(r"Filter: tcp\.stream eq (\d+)", line)
        if header:
            current = sessions.get(int(header.group(1)))
            continue
        node = re.fullmatch(r"Node ([01]): (\S+):(\d+)", line)
        if node and current is not None:
            if (node.group(2), int(node.group(3))) == current.client:
                client_node = int(node.group(1))
            continue
        if current is None or not re.fullmatch(r"\t?[0-9a-f]+", line):
            continue
        # Node 1 is indented in the output.
        from_client = line.startswith("\t") == (client_node == 1)
        current.inside.append((from_client, bytes.fromhex(line.strip())))


def count_smtp(base: list[str], sessions: dict[int, Session]) -> None:
    """Count the SMTP commands, replies, and messages that tshark parses inside TLS."""
    ports = sorted({s.server[1] for s in sessions.values()})
    argv = base[:]
    for port in ports:
        argv += ["-d", f"tls.port=={port},smtp"]
    argv += ["-Y", "smtp || imf", "-T", "fields", "-e", "tcp.stream", "-e", "smtp.req.command",
             "-e", "smtp.response.code", "-e", "imf.message_id"]
    for line in run(argv).splitlines():
        parts = line.split("\t") + ["", "", ""]
        session = sessions.get(int(parts[0]))
        if session is None:
            continue
        if parts[1]:
            session.commands[parts[1]] += 1
        if parts[2]:
            session.replies[parts[2]] += 1
        if parts[3]:
            session.messages += 1


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


def session_frames(session: Session, chunks: list[tuple[bool, bytes]], server_port: int,
                   ident: int) -> list[bytes]:
    """Return the frames of one plain TCP session that carries the chunks."""
    client = session.client
    server = (session.server[0], server_port)
    syn, ack_flag, psh, fin = 0x02, 0x10, 0x08, 0x01
    c_seq, s_seq = 1000, 5000
    out = [
        frame(client, server, c_seq, 0, syn, b"", True, ident),
        frame(server, client, s_seq, c_seq + 1, syn | ack_flag, b"", False, ident + 1),
        frame(client, server, c_seq + 1, s_seq + 1, ack_flag, b"", True, ident + 2),
    ]
    c_seq, s_seq = c_seq + 1, s_seq + 1
    ident += 3
    for from_client, data in chunks:
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


def write_view(path: Path, sessions: list[Session], server_port: int) -> None:
    """Write the plaintext of the sessions as a classic pcap capture."""
    with path.open("wb") as fh:
        fh.write(struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1))
        tick = 0
        ident = 1
        for session in sessions:
            chunks = session.upgraded_in_dialogue() + session.inside
            frames = session_frames(session, chunks, server_port, ident)
            ident += len(frames)
            for data in frames:
                tick += 1000
                fh.write(struct.pack("<IIII", 1_700_000_000 + tick // 1_000_000, tick % 1_000_000,
                                     len(data), len(data)))
                fh.write(data)


def score_view(view: Path, port: int, work: Path, min_ratio: float) -> dict:
    """Score the decrypted view with the realism runner and return its conformance entry."""
    labels = work / "decrypted.labels.yaml"
    labels.write_text(
        "\n".join([
            "labels: fps-realism-labels", "version: 1", f"threshold: {min_ratio}", "sessions:",
            "  - application: smtp_decrypted",
            f"    match: {{transport: tcp, server_port: {port}}}",
            "applications:", "  smtp_decrypted: {ndpi: [SMTP]}",
        ]) + "\n",
        encoding="utf-8",
    )
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


def zeek_on_view(program: str, view: Path, messages: int, attachments: int) -> dict:
    """Run Zeek on the decrypted view and return its row counts and the failed checks."""
    try:
        logs = zeek_check.run_zeek(program, view)
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        raise ToolError(f"zeek: {exc}") from exc
    errors = zeek_check.check(logs, messages, attachments)
    return {
        "smtp_log_rows": len(logs["smtp"]),
        "files_log_rows": len(logs["files"]),
        "weird_log_rows": len(logs["weird"]),
        "errors": errors,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--sessions", type=int, required=True, help="number of STARTTLS sessions in the capture")
    parser.add_argument("--keylog", type=Path, help="key log in the NSS format, when the capture embeds none")
    parser.add_argument("--work", type=Path, help="directory that keeps the decrypted view")
    parser.add_argument("--report", type=Path, help="file for the JSON report")
    parser.add_argument("--zeek", help="Zeek program, to read the decrypted view with Zeek")
    parser.add_argument("--min-ratio", type=float, default=0.99)
    parser.add_argument("--view-port", type=int, default=587, help="server port of the decrypted view")
    parser.add_argument("--tshark", default="tshark")
    args = parser.parse_args(argv)
    try:
        with tempfile.TemporaryDirectory(prefix="fps-smtp-decrypted-") as tmp:
            work = args.work or Path(tmp)
            work.mkdir(parents=True, exist_ok=True)
            base = tshark_args(args.tshark, args.capture, args.keylog)
            sessions = tls_sessions(base)
            if not sessions:
                raise ToolError("the capture has no TLS session")
            plaintext_before(base, sessions)
            follow(base, sessions)
            count_smtp(base, sessions)
            decrypted = [s for s in sessions.values()
                         if any(c for c, _ in s.inside) and any(not c for c, _ in s.inside)]
            started = [s for s in sessions.values() if s.starttls_offered()]
            complete = [
                s for s in decrypted
                if all(s.commands[c] >= 1 for c in REQUIRED_COMMANDS)
                and all(s.replies[c] >= 1 for c in REQUIRED_REPLIES)
                and s.messages == 1
            ]
            view = work / "smtp.decrypted.pcap"
            write_view(view, decrypted, args.view_port)
            entry = score_view(view, args.view_port, work, args.min_ratio)
            zeek = (
                zeek_on_view(args.zeek, view, len(decrypted), len(decrypted))
                if args.zeek else None
            )
    except ToolError as exc:
        print(f"decrypted_check: {exc}", file=sys.stderr)
        return 2
    application = entry["applications"][0] if entry["applications"] else {}
    total = len(sessions)
    result = {
        "capture": args.capture.name,
        "expected_sessions": args.sessions,
        "tls_sessions": total,
        "starttls_offered": len(started),
        "decrypted_sessions": len(decrypted),
        "sessions_with_complete_dialogue": len(complete),
        "smtp_commands": dict(sum((s.commands for s in sessions.values()), Counter())),
        "smtp_replies": dict(sum((s.replies for s in sessions.values()), Counter())),
        "messages": sum(s.messages for s in sessions.values()),
        "server_ports": dict(Counter(s.server[1] for s in sessions.values())),
        "decrypted_view": {
            "classifier": f"{entry['classifier']} {entry['classifier_version']}",
            "sessions": application.get("sessions", 0),
            "smtp": application.get("conforming", 0),
            "ratio": application.get("ratio", 0.0),
            "observed": application.get("observed", {}),
            "tshark_parser_errors": entry.get("parser_errors"),
        },
        "zeek": zeek if zeek is not None else "not run",
        "min_ratio": args.min_ratio,
    }
    result["passed"] = (
        total == args.sessions
        and len(started) == total
        and len(decrypted) == total
        and len(complete) == total
        and result["decrypted_view"]["sessions"] == total
        and result["decrypted_view"]["ratio"] >= args.min_ratio
        and (zeek is None or not zeek["errors"])
    )
    if args.report:
        args.report.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    view_info = result["decrypted_view"]
    zeek_text = (
        "Zeek not run"
        if zeek is None
        else f"Zeek logs {zeek['smtp_log_rows']} messages and {zeek['files_log_rows']} files "
             f"with {len(zeek['errors'])} failed checks"
    )
    print(
        f"decrypted_check: {len(decrypted)} of {total} TLS sessions decrypt (expected {args.sessions}), "
        f"tshark parses the whole SMTP dialogue and {result['messages']} messages in "
        f"{len(complete)} of them, {view_info['classifier']} classifies {view_info['smtp']} of "
        f"{view_info['sessions']} decrypted sessions as SMTP (ratio {view_info['ratio']}, needs "
        f"{args.min_ratio}), {zeek_text}: {'passed' if result['passed'] else 'FAILED'}",
        file=sys.stderr,
    )
    if zeek is not None:
        for error in zeek["errors"]:
            print(f"decrypted_check: zeek FAIL {error}", file=sys.stderr)
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
