# SPDX-License-Identifier: GPL-3.0-only
"""HTTP/1.1 requests and responses of the sessions of a capture (RLM-22).

The packet gap targets that select requests by their path, and the rung byte targets of
the temporal check, need the requests of a keepalive connection with their times and the
bytes of their responses. This module reassembles the two directions of a TCP session from
the segments of the capture, reads the requests (RFC 9112) out of the client direction and
the responses out of the server direction, and pairs them in order.

A session that this reader cannot follow, because a segment is missing, a request is not a
``GET``, or a response has no ``Content-Length``, is not
guessed at. It comes back with an ``error`` in words, and the check that needs it fails
with that reason, as ``hls_check.py`` of the hls module did. The time of a request is the
time of the TCP segment that carries the first octet of its request line.
"""

from __future__ import annotations

import bisect
import re
from collections.abc import Iterable, Sequence
from dataclasses import dataclass, field
from pathlib import Path

from realism.capture import Session, SessionKey, decode_segment, read_frames

_REQUEST_LINE = re.compile(rb"^GET (\S+) HTTP/1\.1\r\n")
_CONTENT_LENGTH = re.compile(r"(?im)^content-length:\s*(\d+)\s*$")


@dataclass(frozen=True)
class Exchange:
    """One request with its response.

    ``wire_bytes`` counts the octets of the response head and body, and ``body_bytes`` the
    ``Content-Length`` of the body.
    """

    time: float
    target: str
    wire_bytes: int
    body_bytes: int


@dataclass
class SessionExchanges:
    """The exchanges of one session, or the reason that they cannot be read."""

    session: Session
    exchanges: list[Exchange] = field(default_factory=list)
    error: str | None = None


def read_exchanges(capture: Path, sessions: Iterable[Session]) -> dict[int, SessionExchanges]:
    """Read the exchanges of the given TCP sessions, by session id.

    Only these sessions are reassembled, so a capture with many other sessions costs only
    the time to decode its frames.
    """
    wanted: dict[SessionKey, Session] = {s.key: s for s in sessions if s.transport == "tcp"}
    parts: dict[int, dict[str, list[tuple[int, float, bytes]]]] = {}
    _, frames = read_frames(capture)
    for frame in frames:
        segment = decode_segment(frame)
        if segment is None or segment.packet.transport != "tcp" or not segment.payload:
            continue
        p = segment.packet
        session = wanted.get(SessionKey.of("tcp", p.src_ip, p.src_port, p.dst_ip, p.dst_port))
        if session is None:
            continue
        from_client = (p.src_ip, p.src_port) == (session.client_ip, session.client_port)
        side = parts.setdefault(session.id, {"client": [], "server": []})
        side["client" if from_client else "server"].append(
            (segment.seq, p.timestamp, segment.payload)
        )
    out: dict[int, SessionExchanges] = {}
    for session in wanted.values():
        result = SessionExchanges(session)
        sides = parts.get(session.id)
        if sides is None or not sides["client"]:
            result.error = "no request in the capture"
        else:
            try:
                result.exchanges = _exchanges(sides["client"], sides["server"])
            except ValueError as exc:
                result.error = str(exc)
        out[session.id] = result
    return out


def _stream(
    segments: Sequence[tuple[int, float, bytes]],
) -> tuple[bytes, list[int], list[float]]:
    """Join the segments of one direction in sequence order, once each."""
    seen: dict[int, tuple[float, bytes]] = {}
    base: int | None = None
    for seq, time, payload in segments:
        if base is None:
            base = seq
        seen.setdefault((seq - base) & 0xFFFFFFFF, (time, payload))
    data = bytearray()
    offsets: list[int] = []
    times: list[float] = []
    for rel in sorted(seen):
        time, payload = seen[rel]
        if rel != len(data):
            raise ValueError(f"the stream has a gap or an overlap at octet {len(data)}")
        offsets.append(rel)
        times.append(time)
        data += payload
    return bytes(data), offsets, times


def _exchanges(
    client: Sequence[tuple[int, float, bytes]], server: Sequence[tuple[int, float, bytes]]
) -> list[Exchange]:
    requests_data, offsets, times = _stream(client)
    responses_data, _, _ = _stream(server)
    requests: list[tuple[float, str]] = []
    at = 0
    while at < len(requests_data):
        end = requests_data.find(b"\r\n\r\n", at)
        if end < 0:
            raise ValueError("a request is cut off")
        match = _REQUEST_LINE.match(requests_data[at : end + 4])
        if not match:
            raise ValueError(f"not a GET request: {requests_data[at : at + 40]!r}")
        index = bisect.bisect_right(offsets, at) - 1
        requests.append((times[index], match.group(1).decode("latin-1")))
        at = end + 4
    out: list[Exchange] = []
    at = 0
    for time, target in requests:
        end = responses_data.find(b"\r\n\r\n", at)
        if end < 0:
            raise ValueError(f"the response to {target} is missing")
        head = responses_data[at:end].decode("latin-1")
        found = _CONTENT_LENGTH.search(head)
        if not found:
            raise ValueError(f"the response to {target} has no Content-Length")
        body = int(found.group(1))
        stop = end + 4 + body
        if stop > len(responses_data):
            raise ValueError(f"the response to {target} is cut off")
        out.append(Exchange(time, target, stop - at, body))
        at = stop
    if at != len(responses_data):
        raise ValueError("the server wrote bytes after the last response")
    return out


@dataclass
class Run:
    """The exchanges of one kind that form one stream: one session, or a joined run."""

    name: str
    session: Session
    sessions: int
    exchanges: list[Exchange]

    @property
    def times(self) -> list[float]:
        """The request times in order."""
        return [e.time for e in self.exchanges]


def describe(session: Session) -> str:
    """Name a session by its ends and its id."""
    return (
        f"{session.transport} session {session.client_ip}:{session.client_port} to "
        f"{session.server_ip}:{session.server_port} (session {session.id})"
    )


def first_error(read: Iterable[SessionExchanges]) -> str | None:
    """Return the reason that the first unreadable session gives, naming the session."""
    for item in read:
        if item.error is not None:
            return f"{describe(item.session)} cannot be read as HTTP/1.1: {item.error}"
    return None


def runs(
    read: Sequence[SessionExchanges], pattern: re.Pattern[str], join: float | None
) -> list[Run]:
    """Select the exchanges whose request target matches, and group them into streams.

    Without ``join`` every session is a stream of its own (an empty one when it holds no
    matching request). With ``join`` (seconds) the matching exchanges of all sessions, in
    time order, form one stream until the gap to the next one exceeds it, which is a
    playback whose segments each use a connection. Sessions come in the order of their ids.
    """
    ordered = sorted(read, key=lambda item: item.session.id)
    selected = [(item, [e for e in item.exchanges if pattern.search(e.target)]) for item in ordered]
    if join is None:
        return [
            Run(describe(item.session), item.session, 1, matching) for item, matching in selected
        ]
    pooled = sorted(
        ((e.time, e, item.session) for item, matching in selected for e in matching),
        key=lambda entry: entry[0],
    )
    out: list[Run] = []
    members: set[int] = set()
    for time, exchange, session in pooled:
        last = out[-1].exchanges[-1].time if out else None
        if last is None or time - last > join:
            out.append(Run("", session, 0, []))
            members = set()
        run = out[-1]
        run.exchanges.append(exchange)
        if session.id not in members:
            members.add(session.id)
            run.sessions = len(members)
    for run in out:
        s = run.session
        run.name = (
            f"run of {run.sessions} sessions of {s.server_ip}:{s.server_port} from "
            f"{s.client_ip}:{s.client_port}"
        )
    return out
