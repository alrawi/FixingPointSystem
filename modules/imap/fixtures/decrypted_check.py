#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Measures the IMAP inside the STARTTLS sessions of the imap fixture.

The sessions of the scenario ``starttls_mailbox`` start in plain text, move to
TLS after ``STARTTLS``, and carry the login and the mailbox dialogue inside TLS.
nDPI reads only the plaintext start of such a session, and Zeek has no way to
decrypt TLS (it takes no key log), so neither tool can see the mailbox dialogue
on the capture. This script decrypts the capture with tshark and the keys that
the capture embeds (or a key log from ``--keylog``) and then measures three
things.

1. tshark parses the IMAP dialogue inside every TLS session of the capture itself
   (it decodes the TLS port as IMAP), so each session shows ``CAPABILITY`` after
   the upgrade, ``LOGIN``, ``SELECT``, ``FETCH``, and ``LOGOUT`` with tagged
   ``OK`` answers, and the follow stream of each session holds at least one
   downloaded RFC 5322 message in a ``BODY[]`` literal.
2. nDPI classifies at least ``--min-ratio`` of the sessions of a decrypted view as
   IMAP. The view is a capture of plain TCP sessions with the addresses of the
   original sessions, on port ``--view-port`` (143, the port of the scenario),
   whose payload is the plaintext that both sides exchanged inside TLS, which is
   what a decrypting proxy shows. The plaintext before the upgrade stays out.
   nDPI 4.2.0 gives IMAPS to a stream that holds the ``STARTTLS`` exchange, even
   with plain text after it, and gives no label to the pre-upgrade lines joined
   to the dialogue without that exchange, while the dialogue inside TLS alone is
   IMAP. The view is synthetic, built from the output of tshark.
3. With ``--zeek``, Zeek reads the view. Zeek ships no IMAP analyzer, so the check
   is conservative: ``conn.log`` must hold one TCP connection per session with
   bytes in both directions, and ``weird.log`` must hold no entry of the IMAP
   analyzer or a TCP parse error. If an ``imap.log`` appears (a Zeek with an IMAP
   package), it must not hold more rows than sessions. The script reports the
   ``service`` that Zeek gives to the view.

    decrypted_check.py CAPTURE --sessions N [--keylog FILE] [--work DIR]
                       [--report FILE] [--zeek PROGRAM] [--min-ratio 0.99]
                       [--view-port 143] [--tshark PROGRAM]

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

MSS = 1400
CLIENT_MAC = bytes.fromhex("020000000001")
SERVER_MAC = bytes.fromhex("020000000002")
REQUIRED_COMMANDS = ("capability", "login", "select", "fetch", "logout")
STARTTLS_COMMAND = re.compile(rb"\S+ STARTTLS", re.IGNORECASE)
LITERAL = re.compile(rb" BODY\[\] \{(\d+)\}\r\n")


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
        """True when the greeting offered STARTTLS, the client asked, and the server agreed."""
        server = b"".join(b for from_client, b in self.before if not from_client)
        client = b"".join(b for from_client, b in self.before if from_client)
        return (
            b" STARTTLS" in server
            and any(STARTTLS_COMMAND.fullmatch(line) for line in client.split(b"\r\n"))
            and b" OK Begin TLS negotiation now." in server
        )

    def downloaded_messages(self) -> int:
        """Count the BODY[] literals of the server inside TLS that hold an RFC 5322 message."""
        server = b"".join(b for from_client, b in self.inside if not from_client)
        count = 0
        for found in LITERAL.finditer(server):
            size = int(found.group(1))
            body = server[found.end():found.end() + size]
            head = body.split(b"\r\n\r\n", 1)[0]
            if len(body) == size and b"\r\nSubject: " in b"\r\n" + head:
                count += 1
        return count


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


def count_imap(base: list[str], sessions: dict[int, Session]) -> None:
    """Count the IMAP commands and tagged answers that tshark parses inside TLS."""
    ports = sorted({s.server[1] for s in sessions.values()})
    argv = base[:]
    for port in ports:
        argv += ["-d", f"tls.port=={port},imap"]
    argv += ["-Y", "imap", "-T", "fields", "-e", "tcp.stream", "-e", "imap.request.command",
             "-e", "imap.response.status"]
    for line in run(argv).splitlines():
        parts = line.split("\t") + ["", ""]
        session = sessions.get(int(parts[0]))
        if session is None:
            continue
        for command in filter(None, parts[1].split(",")):
            session.commands[command.lower()] += 1
        for status in filter(None, parts[2].split(",")):
            session.replies[status.upper()] += 1
    for session in sessions.values():
        session.messages = session.downloaded_messages()


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
            chunks = session.inside
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
            "  - application: imap_decrypted",
            f"    match: {{transport: tcp, server_port: {port}}}",
            "applications:", "  imap_decrypted: {ndpi: [IMAP]}",
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


def read_log(path: Path) -> list[dict]:
    """Return the rows of a JSON Zeek log, or an empty list when it is missing."""
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return []
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def zeek_on_view(program: str, view: Path, sessions: int, port: int) -> dict:
    """Run Zeek on the decrypted view and return its row counts and the failed checks."""
    try:
        with tempfile.TemporaryDirectory(prefix="fps-imap-zeek-") as tmp:
            cwd = Path(tmp)
            argv = [program, "-C", "-r", str(view.resolve()), "LogAscii::use_json=T"]
            proc = subprocess.run(argv, cwd=cwd, capture_output=True, text=True,
                                  timeout=300, check=False)
            if proc.returncode != 0:
                raise RuntimeError(f"exited with {proc.returncode}: {proc.stderr.strip()[-300:]}")
            conn = [r for r in read_log(cwd / "conn.log") if r.get("id.resp_p") == port]
            weird = read_log(cwd / "weird.log")
            imap_rows = read_log(cwd / "imap.log")
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        raise ToolError(f"zeek: {exc}") from exc
    errors: list[str] = []
    if len(conn) != sessions:
        errors.append(f"conn.log: {len(conn)} connections on port {port}, expected {sessions}")
    for i, row in enumerate(conn):
        if not row.get("orig_bytes") or not row.get("resp_bytes"):
            errors.append(f"conn.log row {i + 1} has no bytes in one direction")
    bad = [w for w in weird
           if str(w.get("name", "")).lower().startswith(("imap", "tcp_", "data_before"))]
    for w in bad:
        errors.append(f"weird.log: {w.get('name')} on {w.get('id.orig_h')} to {w.get('id.resp_h')}")
    if len(imap_rows) > sessions:
        errors.append(f"imap.log: {len(imap_rows)} rows for {sessions} sessions")
    return {
        "conn_log_rows": len(conn),
        "imap_log_rows": len(imap_rows),
        "weird_log_rows": len(weird),
        "services": sorted({str(r.get("service")) for r in conn}),
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
    parser.add_argument("--view-port", type=int, default=143, help="server port of the decrypted view")
    parser.add_argument("--tshark", default="tshark")
    args = parser.parse_args(argv)
    try:
        with tempfile.TemporaryDirectory(prefix="fps-imap-decrypted-") as tmp:
            work = args.work or Path(tmp)
            work.mkdir(parents=True, exist_ok=True)
            base = tshark_args(args.tshark, args.capture, args.keylog)
            sessions = tls_sessions(base)
            if not sessions:
                raise ToolError("the capture has no TLS session")
            plaintext_before(base, sessions)
            follow(base, sessions)
            count_imap(base, sessions)
            decrypted = [s for s in sessions.values()
                         if any(c for c, _ in s.inside) and any(not c for c, _ in s.inside)]
            started = [s for s in sessions.values() if s.starttls_offered()]
            complete = [
                s for s in decrypted
                if all(s.commands[c] >= 1 for c in REQUIRED_COMMANDS)
                and s.replies["OK"] >= len(REQUIRED_COMMANDS)
                and s.messages >= 1
            ]
            view = work / "imap.decrypted.pcap"
            write_view(view, decrypted, args.view_port)
            entry = score_view(view, args.view_port, work, args.min_ratio)
            zeek = (
                zeek_on_view(args.zeek, view, len(decrypted), args.view_port)
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
        "imap_commands": dict(sum((s.commands for s in sessions.values()), Counter())),
        "imap_statuses": dict(sum((s.replies for s in sessions.values()), Counter())),
        "messages": sum(s.messages for s in sessions.values()),
        "server_ports": dict(Counter(s.server[1] for s in sessions.values())),
        "decrypted_view": {
            "classifier": f"{entry['classifier']} {entry['classifier_version']}",
            "sessions": application.get("sessions", 0),
            "imap": application.get("conforming", 0),
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
        else f"Zeek logs {zeek['conn_log_rows']} connections (services {zeek['services']}) and "
             f"{zeek['imap_log_rows']} imap rows with {len(zeek['errors'])} failed checks"
    )
    print(
        f"decrypted_check: {len(decrypted)} of {total} TLS sessions decrypt (expected {args.sessions}), "
        f"tshark parses the whole IMAP dialogue and {result['messages']} downloaded messages in "
        f"{len(complete)} of them, {view_info['classifier']} classifies {view_info['imap']} of "
        f"{view_info['sessions']} decrypted sessions as IMAP (ratio {view_info['ratio']}, needs "
        f"{args.min_ratio}), {zeek_text}: {'passed' if result['passed'] else 'FAILED'}",
        file=sys.stderr,
    )
    if zeek is not None:
        for error in zeek["errors"]:
            print(f"decrypted_check: zeek FAIL {error}", file=sys.stderr)
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
