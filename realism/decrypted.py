# SPDX-License-Identifier: GPL-3.0-only
"""The decrypted view of a capture, and its scoring with the realism checks.

nDPI and Zeek cannot read inside TLS, so on the capture itself they name every
TLS session ``TLS`` and cannot say whether HTTP, SMTP, or IMAP runs inside. This
module decrypts the capture with tshark and the keys that the capture embeds (or
a key log), writes the plaintext of every TLS session as a capture of plain TCP
sessions with the addresses and ports of the originals, and scores that
*decrypted view* with :func:`realism.report.score`, so nDPI and the parsers read
what a decrypting proxy would show. The view is synthetic: tshark supplies its
bytes, and the runner frames them as TCP segments with correct checksums.

Two plaintexts exist for a session. ``inside`` is what both sides sent after the
ClientHello. ``upgrade`` is for STARTTLS dialogues: it puts what they sent
before the ClientHello in front, without the ``STARTTLS`` command and its reply,
because nDPI and Zeek stop reading a stream at that command.
"""

from __future__ import annotations

import ipaddress
import os
import re
import struct
import subprocess
from collections import Counter
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from realism.classifier import Classifier
from realism.parsers import ParserRunner
from realism.report import score

DEFAULT_TSHARK = "tshark"
DEFAULT_TIMEOUT_S = 300.0
PLAINTEXTS = ("inside", "upgrade")
MSS = 1400
CLIENT_MAC = bytes.fromhex("020000000001")
SERVER_MAC = bytes.fromhex("020000000002")
_STARTTLS = re.compile(rb"(?:\S+ )?(?:STARTTLS|STLS)", re.IGNORECASE)

Chunk = tuple[bool, bytes]
Endpoint = tuple[str, int]


class DecryptedError(Exception):
    """The decrypted view could not be built, or a tool failed or printed nonsense."""


@dataclass
class TlsSession:
    """One TLS session of a capture, with its plaintext in order.

    A chunk is ``(from_client, bytes)``. ``before`` holds what the two sides sent
    before the ClientHello (the plain start of a STARTTLS dialogue), and
    ``inside`` what they sent inside TLS.
    """

    stream: int
    hello_frame: int
    client: Endpoint
    server: Endpoint
    before: list[Chunk] = field(default_factory=list)
    inside: list[Chunk] = field(default_factory=list)

    @property
    def decrypted(self) -> bool:
        """True when tshark read plaintext of both sides inside TLS."""
        return any(c for c, _ in self.inside) and any(not c for c, _ in self.inside)

    def chunks(self, plaintext: str) -> list[Chunk]:
        """Return the plaintext that the view carries for this session."""
        if plaintext == "upgrade":
            return drop_starttls(self.before) + self.inside
        return list(self.inside)


def drop_starttls(chunks: Sequence[Chunk]) -> list[Chunk]:
    """Remove the STARTTLS (or STLS) command of the client and the reply that follows it."""
    out: list[Chunk] = []
    skip_reply = False
    for from_client, data in chunks:
        if from_client and _STARTTLS.fullmatch(data.strip()):
            skip_reply = True
            continue
        if skip_reply and not from_client:
            skip_reply = False
            continue
        out.append((from_client, data))
    return out


class Tshark:
    """Run tshark out of process to read the TLS sessions of a capture."""

    def __init__(self, program: str | None = None, timeout_s: float = DEFAULT_TIMEOUT_S) -> None:
        """Create a runner for a tshark path, default ``FPS_TSHARK`` and then ``tshark``."""
        self.program = program or os.environ.get("FPS_TSHARK") or DEFAULT_TSHARK
        self.timeout_s = timeout_s

    def run(self, capture: Path, keylog: Path | None, args: Sequence[str]) -> str:
        """Run tshark on the capture and return its standard output."""
        argv = [self.program, "-r", str(capture)]
        if keylog is not None:
            argv += ["-o", f"tls.keylog_file:{keylog}"]
        argv += list(args)
        try:
            proc = subprocess.run(
                argv,
                capture_output=True,
                text=True,
                timeout=self.timeout_s,
                check=False,
                stdin=subprocess.DEVNULL,
            )
        except FileNotFoundError as exc:
            raise DecryptedError(
                f"{self.program} not found, install tshark or set FPS_TSHARK"
            ) from exc
        except subprocess.TimeoutExpired as exc:
            raise DecryptedError(f"{self.program} ran longer than {self.timeout_s} s") from exc
        if proc.returncode != 0:
            tail = proc.stderr.strip().splitlines()[-1:]
            raise DecryptedError(
                f"{self.program} exited with {proc.returncode}: "
                + (tail[0] if tail else "no output")
            )
        return proc.stdout


def parse_hellos(text: str) -> dict[int, TlsSession]:
    """Read the ClientHello listing into sessions by TCP stream index.

    The listing has the tab separated fields ``tcp.stream``, ``frame.number``,
    ``ip.src``, ``ipv6.src``, ``tcp.srcport``, ``ip.dst``, ``ipv6.dst``, and
    ``tcp.dstport``. The first ClientHello of a stream names the session.
    """
    sessions: dict[int, TlsSession] = {}
    for line in text.splitlines():
        parts = line.split("\t")
        if len(parts) != 8:
            raise DecryptedError(f"tshark printed a ClientHello row with {len(parts)} fields")
        stream, frame, src4, src6, sport, dst4, dst6, dport = parts
        try:
            session = TlsSession(
                int(stream), int(frame), (src4 or src6, int(sport)), (dst4 or dst6, int(dport))
            )
            ipaddress.ip_address(session.client[0])
            ipaddress.ip_address(session.server[0])
        except ValueError as exc:
            raise DecryptedError(f"tshark printed an unreadable ClientHello row: {line!r}") from exc
        sessions.setdefault(session.stream, session)
    return sessions


def parse_before(text: str, sessions: dict[int, TlsSession]) -> None:
    """Fill ``before`` from the TCP payload listing of the frames ahead of each ClientHello.

    The listing has the fields ``frame.number``, ``tcp.stream``, ``ip.src``,
    ``ipv6.src``, ``tcp.srcport``, ``tcp.seq``, and ``tcp.payload``. A
    retransmitted segment counts once.
    """
    seen: set[tuple[int, bool, int]] = set()
    for line in text.splitlines():
        parts = line.split("\t")
        if len(parts) != 7:
            raise DecryptedError(f"tshark printed a payload row with {len(parts)} fields")
        frame, stream, src4, src6, sport, seq, payload = parts
        session = sessions.get(int(stream))
        if session is None or int(frame) >= session.hello_frame or not payload:
            continue
        from_client = (src4 or src6, int(sport)) == session.client
        key = (session.stream, from_client, int(seq))
        if key in seen:
            continue
        seen.add(key)
        session.before.append((from_client, bytes.fromhex(payload.replace(":", ""))))


def parse_follow(text: str, sessions: dict[int, TlsSession]) -> None:
    """Fill ``inside`` from the output of ``-z follow,tls,raw,N`` for several streams.

    Each stream starts with ``Filter: tcp.stream eq N`` and two ``Node`` lines that
    name the endpoints. Node 1 is indented in the data lines, and which node is the
    client comes from those lines, because in TLS 1.3 the first plaintext may come
    from either side.
    """
    current: TlsSession | None = None
    client_node = 1
    for line in text.splitlines():
        header = re.fullmatch(r"Filter: tcp\.stream eq (\d+)", line)
        if header:
            current = sessions.get(int(header.group(1)))
            client_node = 1
            continue
        node = re.fullmatch(r"Node ([01]): (\S+):(\d+)", line)
        if node and current is not None:
            if (node.group(2), int(node.group(3))) == current.client:
                client_node = int(node.group(1))
            continue
        if current is None or not re.fullmatch(r"\t?[0-9a-f]+", line):
            continue
        from_client = line.startswith("\t") == (client_node == 1)
        current.inside.append((from_client, bytes.fromhex(line.strip())))


def read_sessions(
    capture: Path, keylog: Path | None, tshark: Tshark, plaintext: str = "inside"
) -> list[TlsSession]:
    """Decrypt the capture with tshark and return its TLS sessions in stream order."""
    fields = (
        "tcp.stream",
        "frame.number",
        "ip.src",
        "ipv6.src",
        "tcp.srcport",
        "ip.dst",
        "ipv6.dst",
        "tcp.dstport",
    )
    hello = ["-Y", "tls.handshake.type == 1", "-T", "fields"]
    for name in fields:
        hello += ["-e", name]
    sessions = parse_hellos(tshark.run(capture, keylog, hello))
    if not sessions:
        return []
    if plaintext == "upgrade":
        payload = ["-Y", "tcp.len > 0", "-T", "fields"]
        for name in (
            "frame.number",
            "tcp.stream",
            "ip.src",
            "ipv6.src",
            "tcp.srcport",
            "tcp.seq",
            "tcp.payload",
        ):
            payload += ["-e", name]
        parse_before(tshark.run(capture, keylog, payload), sessions)
    follow = ["-q"]
    for stream in sessions:
        follow += ["-z", f"follow,tls,raw,{stream}"]
    parse_follow(tshark.run(capture, keylog, follow), sessions)
    return [sessions[stream] for stream in sorted(sessions)]


def _checksum(data: bytes) -> int:
    if len(data) % 2:
        data += b"\0"
    total = sum(struct.unpack(f"!{len(data) // 2}H", data))
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return int(~total & 0xFFFF)


def _frame(
    src: Endpoint,
    dst: Endpoint,
    seq: int,
    ack: int,
    flags: int,
    payload: bytes,
    from_client: bool,
    ident: int,
) -> bytes:
    """Build an Ethernet, IP, and TCP frame with correct checksums."""
    src_ip, dst_ip = ipaddress.ip_address(src[0]), ipaddress.ip_address(dst[0])
    if src_ip.version != dst_ip.version:
        raise DecryptedError(f"session between {src[0]} and {dst[0]} mixes IP versions")
    tcp = struct.pack("!HHIIBBHHH", src[1], dst[1], seq, ack, 5 << 4, flags, 65535, 0, 0) + payload
    if src_ip.version == 4:
        pseudo = src_ip.packed + dst_ip.packed + struct.pack("!BBH", 0, 6, len(tcp))
    else:
        pseudo = src_ip.packed + dst_ip.packed + struct.pack("!I", len(tcp)) + b"\0\0\0\x06"
    tcp = tcp[:16] + struct.pack("!H", _checksum(pseudo + tcp)) + tcp[18:]
    if src_ip.version == 4:
        header = struct.pack("!BBHHHBBH", 0x45, 0, 20 + len(tcp), ident & 0xFFFF, 0x4000, 64, 6, 0)
        header += src_ip.packed + dst_ip.packed
        header = header[:10] + struct.pack("!H", _checksum(header)) + header[12:]
        ethertype = b"\x08\x00"
    else:
        header = struct.pack("!IHBB", 0x60000000, len(tcp), 6, 64) + src_ip.packed + dst_ip.packed
        ethertype = b"\x86\xdd"
    macs = (SERVER_MAC + CLIENT_MAC) if from_client else (CLIENT_MAC + SERVER_MAC)
    return macs + ethertype + header + tcp


def session_frames(
    client: Endpoint, server: Endpoint, chunks: Sequence[Chunk], ident: int
) -> list[bytes]:
    """Return the frames of one plain TCP session that carries the chunks."""
    syn, ack_flag, psh, fin = 0x02, 0x10, 0x08, 0x01
    c_seq, s_seq = 1000, 5000
    out = [
        _frame(client, server, c_seq, 0, syn, b"", True, ident),
        _frame(server, client, s_seq, c_seq + 1, syn | ack_flag, b"", False, ident + 1),
        _frame(client, server, c_seq + 1, s_seq + 1, ack_flag, b"", True, ident + 2),
    ]
    c_seq, s_seq = c_seq + 1, s_seq + 1
    ident += 3
    for from_client, data in chunks:
        for start in range(0, len(data), MSS):
            piece = data[start : start + MSS]
            if from_client:
                out.append(_frame(client, server, c_seq, s_seq, ack_flag | psh, piece, True, ident))
                c_seq += len(piece)
            else:
                out.append(
                    _frame(server, client, s_seq, c_seq, ack_flag | psh, piece, False, ident)
                )
                s_seq += len(piece)
            ident += 1
    out.append(_frame(client, server, c_seq, s_seq, fin | ack_flag, b"", True, ident))
    out.append(_frame(server, client, s_seq, c_seq + 1, fin | ack_flag, b"", False, ident + 1))
    return out


def write_view(
    path: Path,
    sessions: Sequence[TlsSession],
    plaintext: str = "inside",
    view_port: int | None = None,
) -> int:
    """Write the plaintext of the sessions as a classic pcap and return the frame count.

    Every session keeps its addresses and client port. The server port is the one of
    the session unless ``view_port`` names another, which lets a view put sessions on
    the well known port of their protocol.
    """
    count = 0
    ident = 1
    tick = 0
    try:
        with path.open("wb") as out:
            out.write(struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1))
            for session in sessions:
                server = (session.server[0], view_port or session.server[1])
                frames = session_frames(session.client, server, session.chunks(plaintext), ident)
                ident += len(frames)
                for data in frames:
                    tick += 1000
                    out.write(
                        struct.pack(
                            "<IIII",
                            1_700_000_000 + tick // 1_000_000,
                            tick % 1_000_000,
                            len(data),
                            len(data),
                        )
                    )
                    out.write(data)
                    count += 1
    except OSError as exc:
        raise DecryptedError(f"write {path}: {exc.strerror}") from exc
    return count


@dataclass
class DecryptedResult:
    """The outcome of scoring a decrypted view."""

    capture: str
    plaintext: str
    tls_sessions: list[TlsSession]
    expected_sessions: int | None
    report: dict[str, Any]
    view: str | None = None

    @property
    def decrypted_sessions(self) -> int:
        """Number of TLS sessions with plaintext of both sides."""
        return sum(1 for session in self.tls_sessions if session.decrypted)

    @property
    def failures(self) -> list[str]:
        """Return why the view check fails, empty when it passes."""
        total = len(self.tls_sessions)
        reasons: list[str] = []
        if total == 0:
            reasons.append("the capture has no TLS session")
        elif self.decrypted_sessions != total:
            reasons.append(f"only {self.decrypted_sessions} of {total} TLS sessions decrypt")
        if self.expected_sessions is not None and total != self.expected_sessions:
            reasons.append(
                f"the capture has {total} TLS sessions, expected {self.expected_sessions}"
            )
        if not self.report["passed"]:
            failed = [c["check"] for c in self.report["checks"] if not c["passed"]]
            reasons.append("the view fails " + (", ".join(failed) or "every check"))
        return reasons

    def _conformance(self) -> dict[str, Any]:
        entry: dict[str, Any] = next(
            (c for c in self.report["checks"] if c["check"] == "conformance"), {}
        )
        return entry

    @property
    def passed(self) -> bool:
        """True when every TLS session decrypts and the view passes every check."""
        return not self.failures

    def to_dict(self) -> dict[str, Any]:
        """Return the result as a JSON compatible mapping."""
        conformance = self._conformance()
        return {
            "capture": self.capture,
            "plaintext": self.plaintext,
            "passed": self.passed,
            "failures": self.failures,
            "tls_sessions": len(self.tls_sessions),
            "expected_sessions": self.expected_sessions,
            "decrypted_sessions": self.decrypted_sessions,
            "server_ports": dict(Counter(s.server[1] for s in self.tls_sessions)),
            "view": self.view,
            "classifier": " ".join(
                str(conformance.get(key)) for key in ("classifier", "classifier_version")
            ),
            "report": self.report,
        }

    def summary(self) -> str:
        """Return the one line summary for standard error."""
        conformance = self._conformance()
        apps = ", ".join(
            f"{a['application']} {a['conforming']}/{a['sessions']}"
            for a in conformance.get("applications", [])
        )
        parsers = next((c for c in self.report["checks"] if c["check"] == "parser_errors"), None)
        errors = "not run" if parsers is None else f"{parsers['errors_total']} errors"
        return (
            f"realism decrypted: {self.decrypted_sessions} of {len(self.tls_sessions)} TLS "
            f"sessions decrypt, {conformance.get('classifier')} "
            f"{conformance.get('classifier_version')} classifies the view as {apps or 'nothing'} "
            f"(needs {conformance.get('threshold')}), parsers {errors}: "
            f"{'passed' if self.passed else 'FAILED'}"
        )


def run(
    capture: Path,
    labels: Path,
    classifier: Classifier,
    work: Path,
    tshark: Tshark | None = None,
    keylog: Path | None = None,
    plaintext: str = "inside",
    view_port: int | None = None,
    expected_sessions: int | None = None,
    keep_view: Path | None = None,
    threshold: float | None = None,
    parsers: Sequence[ParserRunner] | None = None,
    missing_parsers: Sequence[str] = (),
    skip_reasons: Mapping[str, str] | None = None,
) -> DecryptedResult:
    """Decrypt the capture, write its decrypted view, and score the view.

    ``labels`` describes the *view*: its session rules match the ports of the view
    (the ports of the TLS sessions, or ``view_port``) and its applications name the
    protocol inside TLS, for example ``{ndpi: [HTTP]}``. ``work`` is a directory for
    the view when ``keep_view`` is None. The score is the realism report of the view
    with the conformance check and, with ``parsers``, the parser error check.
    """
    if plaintext not in PLAINTEXTS:
        raise DecryptedError(f"plaintext must be one of {', '.join(PLAINTEXTS)}")
    sessions = read_sessions(capture, keylog, tshark or Tshark(), plaintext)
    view = keep_view or work / f"{capture.stem}.decrypted.pcap"
    decrypted = [session for session in sessions if session.decrypted]
    write_view(view, decrypted, plaintext, view_port)
    report = score(
        view,
        labels,
        classifier,
        threshold=threshold,
        parsers=parsers,
        missing_parsers=missing_parsers,
        skip_reasons=skip_reasons,
    )
    return DecryptedResult(
        capture=capture.name,
        plaintext=plaintext,
        tls_sessions=sessions,
        expected_sessions=expected_sessions,
        report=report.to_dict(),
        view=str(keep_view) if keep_view else None,
    )
