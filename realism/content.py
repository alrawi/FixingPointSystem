# SPDX-License-Identifier: GPL-3.0-only
"""Content variety check of the realism runner.

Traffic that repeats the same bodies, or that sends payloads of one fixed size, does
not look like the application it imitates, and a middlebox that deduplicates or
caches would treat it differently. The check reassembles the application payloads of
every session, counts exact duplicates, and describes the size distribution.

**Units.** A payload unit is one of

* an HTTP/1 message, request or response, when the labels name an HTTP application
  (an application whose name starts with ``http``) or the client stream starts with
  an HTTP method. The message is split with ``Content-Length`` or chunked framing.
* one UDP datagram.
* one whole direction of a TCP session for any other protocol.

Units of zero bytes do not count. ``fps-realism`` reads the capture itself and does
not use :mod:`capture`, so the realism runner stays independent of the extractor.

**Normalization.** The digest of a unit is the SHA-256 of its normalized form, so two
units that differ only in a field that varies by design are duplicates.

* HTTP: the start line, the headers in wire order with lower case names and trimmed
  values, and the body after the chunked framing is removed. Headers that vary per
  message without changing the content are dropped, see :data:`IGNORED_HEADERS`, and
  the labels can add names with ``content.ignore_headers``. ``Content-Length`` stays
  in, because it follows from the body. A message with an empty body still counts,
  with its start line and headers, so ten identical ``GET /`` requests are ten
  copies of one unit.
* DNS (application ``dns``): the datagram without its 2 byte transaction identifier.
* Everything else: the bytes as they are.

The duplicate ratio of a group is ``(units - distinct digests) / units``. Groups are
the units of one application in one direction, so a request never counts as a copy of
a response. The check fails when any application exceeds ``max_duplicate_ratio``
(default 1%).

**Sizes.** The size of a unit is its length in bytes on the reassembled stream (the
whole message for HTTP, framing included). The entry reports count, minimum, maximum,
mean, population standard deviation, coefficient of variation (``stddev / mean``) and
the quantiles p5, p25, p50, p75, p95 and p99 by linear interpolation, per application
and per direction. When the labels give ``content.sizes`` for an application, the
observed median, and the observed coefficient of variation when the labels give one,
must lie within the tolerance of the expected values, or the check fails.

**Skipped sessions.** Sessions with a ``tls_profile`` in the labels, and sessions whose
stream starts with a TLS record, are encrypted and skipped, because the encryption
check decrypts only to find Finished messages and does not expose plaintext. They are
counted in ``skipped_encrypted_sessions``. A TCP direction with a hole in its sequence
numbers is cut at the hole and counted in ``incomplete_directions``, and a message cut
short by the end of the stream is dropped and counted in ``truncated_units``. The
check fails when no payload at all could be analysed.
"""

from __future__ import annotations

import hashlib
import math
import re
from collections import defaultdict
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from realism.capture import Segment, Session, SessionKey, decode_segment, read_frames
from realism.labels import ContentLabels, Labels, SizeExpectation
from realism.series import failures_series

CHECK_NAME = "content"
METRIC = "realism.duplicate_ratio"
UNLABELLED = "unlabelled"
MAX_STREAM_BYTES = 16 * 1024 * 1024
MAX_DUPLICATES_LISTED = 20
QUANTILES = (("p5", 0.05), ("p25", 0.25), ("p50", 0.5), ("p75", 0.75), ("p95", 0.95), ("p99", 0.99))

# Headers that differ between messages with the same content: time stamps, cache
# validators, cookies, and trace identifiers.
IGNORED_HEADERS = frozenset(
    {
        "date",
        "expires",
        "last-modified",
        "age",
        "etag",
        "if-modified-since",
        "if-none-match",
        "set-cookie",
        "cookie",
        "x-request-id",
        "request-id",
        "x-trace-id",
        "x-amzn-trace-id",
        "traceparent",
        "tracestate",
        "server-timing",
    }
)

_HTTP_METHODS = (
    b"GET ",
    b"POST ",
    b"PUT ",
    b"DELETE ",
    b"HEAD ",
    b"OPTIONS ",
    b"PATCH ",
    b"CONNECT ",
    b"TRACE ",
)
_REQUEST_LINE = re.compile(rb"^[A-Z]+ \S+ HTTP/\d\.\d$")
_STATUS_LINE = re.compile(rb"^HTTP/\d\.\d \d{3}( .*)?$")
_TCP_SYN = 0x02
_HALF = 1 << 31
_MOD = 1 << 32
_CLIENT, _SERVER = 0, 1
_DIRECTION_NAMES = ("client", "server")


@dataclass
class _Half:
    """The packets of one direction of one session."""

    segments: list[tuple[int, bytes]] = field(default_factory=list)
    datagrams: list[bytes] = field(default_factory=list)
    syn_seq: int | None = None
    stored: int = 0


@dataclass
class _Flow:
    """Both directions of one session, the client direction first."""

    halves: tuple[_Half, _Half] = field(default_factory=lambda: (_Half(), _Half()))


@dataclass(frozen=True)
class Unit:
    """One payload unit with its direction, size, and digest."""

    session: int
    application: str
    direction: str
    size: int
    digest: str


@dataclass
class Stats:
    """Counters of the reassembly, for the report."""

    sessions_analysed: int = 0
    skipped_encrypted: int = 0
    without_payload: int = 0
    incomplete_directions: int = 0
    truncated_units: int = 0
    unparsed_tails: int = 0


def read_flows(capture: Path, sessions: Sequence[Session]) -> dict[int, _Flow]:
    """Read the payload bytes of every session of a capture, keyed by session id."""
    by_key: dict[SessionKey, Session] = {s.key: s for s in sessions}
    flows: dict[int, _Flow] = {}
    _, frames = read_frames(capture)
    for frame in frames:
        segment = decode_segment(frame)
        if segment is None:
            continue
        p = segment.packet
        session = by_key.get(SessionKey.of(p.transport, p.src_ip, p.src_port, p.dst_ip, p.dst_port))
        if session is None:
            continue
        from_client = (p.src_ip, p.src_port) == (session.client_ip, session.client_port)
        half = flows.setdefault(session.id, _Flow()).halves[_CLIENT if from_client else _SERVER]
        _store(half, segment)
    return flows


def _store(half: _Half, segment: Segment) -> None:
    packet = segment.packet
    if packet.transport == "udp":
        if segment.payload and half.stored < MAX_STREAM_BYTES:
            half.datagrams.append(segment.payload)
            half.stored += len(segment.payload)
        return
    if packet.tcp_flags & _TCP_SYN and half.syn_seq is None:
        half.syn_seq = (segment.seq + 1) % _MOD
    if segment.payload and half.stored < MAX_STREAM_BYTES:
        half.segments.append((segment.seq, segment.payload))
        half.stored += len(segment.payload)


def reassemble(half: _Half) -> tuple[bytes, bool]:
    """Join the TCP segments of one direction in sequence order.

    The first copy of every byte wins. Offsets count from the byte after the SYN,
    or from the lowest sequence number when the capture missed the SYN. The stream
    ends at the first hole, and the flag says whether the whole direction arrived.
    """
    if not half.segments:
        return b"", True
    anchor = half.syn_seq if half.syn_seq is not None else half.segments[0][0]
    placed = []
    for order, (seq, data) in enumerate(half.segments):
        offset = (seq - anchor) % _MOD
        if offset >= _HALF:
            offset -= _MOD
        placed.append((offset, order, data))
    placed.sort(key=lambda item: (item[0], item[1]))
    if half.syn_seq is None:
        floor = placed[0][0]
        placed = [(o - floor, n, d) for o, n, d in placed]
    out = bytearray()
    for offset, _, data in placed:
        if offset < 0:
            continue
        if offset > len(out):
            return bytes(out), False
        if offset + len(data) > len(out):
            out += data[len(out) - offset :]
    return bytes(out), True


def _looks_encrypted(stream: bytes) -> bool:
    """Return True when a stream starts with a TLS handshake record."""
    return len(stream) >= 3 and stream[0] == 0x16 and stream[1] == 3 and stream[2] <= 4


@dataclass(frozen=True)
class _Message:
    """One HTTP message after framing."""

    size: int
    normalized: bytes
    truncated: bool = False


def split_http(stream: bytes, ignore: frozenset[str]) -> tuple[list[_Message], bool]:
    """Split a stream into HTTP/1 messages and normalize each of them.

    The second value is True when the end of the stream does not parse as a message,
    for example because the stream is not HTTP at all.
    """
    messages: list[_Message] = []
    pos = 0
    while pos < len(stream):
        end = stream.find(b"\r\n\r\n", pos)
        if end < 0:
            return messages, True
        lines = stream[pos:end].split(b"\r\n")
        start_line = lines[0]
        is_response = _STATUS_LINE.match(start_line) is not None
        if not is_response and not _REQUEST_LINE.match(start_line):
            return messages, True
        headers: list[tuple[bytes, bytes]] = []
        for line in lines[1:]:
            name, colon, value = line.partition(b":")
            if not colon or not name.strip():
                return messages, True
            headers.append((name.strip().lower(), value.strip()))
        body_at = end + 4
        body, stop, truncated = _body(stream, body_at, headers, start_line, is_response)
        kept = b"".join(
            name + b":" + value + b"\n"
            for name, value in headers
            if name.decode("latin-1") not in ignore
        )
        normalized = start_line + b"\n" + kept + b"\n" + body
        messages.append(_Message(stop - pos, normalized, truncated))
        pos = stop
    return messages, False


def _body(
    stream: bytes,
    at: int,
    headers: list[tuple[bytes, bytes]],
    start_line: bytes,
    is_response: bool,
) -> tuple[bytes, int, bool]:
    """Return the body, the end offset of the message, and whether the stream cut it short."""
    fields = dict(headers)
    if b"chunked" in fields.get(b"transfer-encoding", b"").lower():
        return _chunked(stream, at)
    length = fields.get(b"content-length", b"")
    if length.isdigit():
        stop = at + int(length)
        return stream[at:stop], min(stop, len(stream)), stop > len(stream)
    if is_response:
        status = int(start_line.split(b" ")[1])
        if not (100 <= status < 200 or status in (204, 304)):
            return stream[at:], len(stream), False
    return b"", at, False


def _chunked(stream: bytes, at: int) -> tuple[bytes, int, bool]:
    out = bytearray()
    pos = at
    while True:
        line_end = stream.find(b"\r\n", pos)
        if line_end < 0:
            return bytes(out), len(stream), True
        size_text = stream[pos:line_end].split(b";")[0].strip()
        try:
            size = int(size_text, 16)
        except ValueError:
            return bytes(out), len(stream), True
        pos = line_end + 2
        if size == 0:
            trailer_end = stream.find(b"\r\n\r\n", line_end)
            if stream[pos : pos + 2] == b"\r\n":
                return bytes(out), pos + 2, False
            if trailer_end < 0:
                return bytes(out), len(stream), True
            return bytes(out), trailer_end + 4, False
        if pos + size + 2 > len(stream):
            return bytes(out), len(stream), True
        out += stream[pos : pos + size]
        pos += size + 2


def _digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _protocol(application: str, client_stream: bytes, transport: str) -> str:
    name = application.lower()
    if name.startswith("http"):
        return "http"
    if name == "dns":
        return "dns"
    if transport == "tcp" and client_stream.startswith(_HTTP_METHODS):
        return "http"
    return "raw"


def collect_units(
    sessions: Sequence[Session],
    flows: Mapping[int, _Flow],
    labels: Labels,
    ignore: frozenset[str],
) -> tuple[list[Unit], Stats]:
    """Turn the flows of the sessions into payload units."""
    units: list[Unit] = []
    stats = Stats()
    for session in sessions:
        flow = flows.get(session.id)
        if flow is None:
            stats.without_payload += 1
            continue
        application = labels.application_of(session) or UNLABELLED
        streams: list[bytes] = []
        for half in flow.halves:
            if session.transport == "tcp":
                data, complete = reassemble(half)
                if not complete:
                    stats.incomplete_directions += 1
                streams.append(data)
            else:
                streams.append(b"")
        if session.transport == "tcp":
            if labels.tls_profile_of(session) is not None or any(
                _looks_encrypted(s) for s in streams
            ):
                stats.skipped_encrypted += 1
                continue
            if not any(streams):
                stats.without_payload += 1
                continue
        elif not any(h.datagrams for h in flow.halves):
            stats.without_payload += 1
            continue
        stats.sessions_analysed += 1
        protocol = _protocol(application, streams[_CLIENT], session.transport)
        for index, half in enumerate(flow.halves):
            direction = _DIRECTION_NAMES[index]
            for size, body in _direction_units(
                session.transport, half, streams[index], protocol, ignore, stats
            ):
                units.append(Unit(session.id, application, direction, size, _digest(body)))
    return units, stats


def _direction_units(
    transport: str,
    half: _Half,
    stream: bytes,
    protocol: str,
    ignore: frozenset[str],
    stats: Stats,
) -> list[tuple[int, bytes]]:
    """Return the size and normalized form of each payload unit of one direction."""
    if transport == "udp":
        if protocol == "dns":
            return [(len(d), d[2:]) for d in half.datagrams if len(d) > 2]
        return [(len(d), d) for d in half.datagrams]
    if not stream:
        return []
    if protocol == "http":
        messages, unparsed = split_http(stream, ignore)
        out = []
        for message in messages:
            if message.truncated:
                stats.truncated_units += 1
            else:
                out.append((message.size, message.normalized))
        if unparsed:
            stats.unparsed_tails += 1
            if not messages:
                out.append((len(stream), stream))
        return out
    return [(len(stream), stream)]


def quantile(values: Sequence[float], q: float) -> float:
    """Return the quantile ``q`` of sorted values by linear interpolation."""
    if len(values) == 1:
        return float(values[0])
    pos = q * (len(values) - 1)
    low = math.floor(pos)
    high = min(low + 1, len(values) - 1)
    return float(values[low] + (values[high] - values[low]) * (pos - low))


def describe(sizes: Sequence[int]) -> dict[str, Any]:
    """Return count, extremes, mean, standard deviation, variation, and quantiles."""
    ordered = sorted(sizes)
    count = len(ordered)
    mean = sum(ordered) / count
    stddev = math.sqrt(sum((x - mean) ** 2 for x in ordered) / count)
    return {
        "count": count,
        "min": ordered[0],
        "max": ordered[-1],
        "mean": round(mean, 3),
        "stddev": round(stddev, 3),
        "cv": round(stddev / mean, 6) if mean > 0 else 0.0,
        "quantiles": {name: round(quantile(ordered, q), 3) for name, q in QUANTILES},
    }


def _group(units: Sequence[Unit]) -> dict[str, Any]:
    distinct = len({u.digest for u in units})
    duplicates = len(units) - distinct
    return {
        "units": len(units),
        "distinct_units": distinct,
        "duplicate_units": duplicates,
        "duplicate_ratio": round(duplicates / len(units), 6),
        "size": describe([u.size for u in units]),
    }


@dataclass
class ContentResult:
    """The outcome of the content variety check."""

    max_duplicate_ratio: float
    ignored_headers: list[str]
    sessions_total: int
    stats: Stats
    units: list[Unit]
    applications: list[dict[str, Any]]
    expected_sizes: list[dict[str, Any]]
    duplicates: list[dict[str, Any]]
    failures: list[str]

    @property
    def duplicate_units(self) -> int:
        """The number of units that repeat an earlier unit of their group."""
        return sum(int(a["duplicate_units"]) for a in self.applications)

    @property
    def ratio(self) -> float:
        """The duplicate share over all units, 0 when there are none."""
        return self.duplicate_units / len(self.units) if self.units else 0.0

    @property
    def passed(self) -> bool:
        """True when nothing in ``failures`` stands against the capture."""
        return not self.failures

    def metrics(self) -> list[dict[str, Any]]:
        """Return the duplicate ratio of each application with payloads, and the failures series."""
        return [
            *(
                {
                    "name": METRIC,
                    "labels": {"application": entry["application"]},
                    "value": entry["duplicate_ratio"],
                }
                for entry in self.applications
            ),
            failures_series(CHECK_NAME, len(self.failures), self.passed),
        ]

    def to_dict(self) -> dict[str, Any]:
        """Return the check entry of the realism report."""
        s = self.stats
        return {
            "check": CHECK_NAME,
            "metrics": self.metrics(),
            "passed": self.passed,
            "max_duplicate_ratio": self.max_duplicate_ratio,
            "ignored_headers": self.ignored_headers,
            "sessions_total": self.sessions_total,
            "sessions_analysed": s.sessions_analysed,
            "skipped_encrypted_sessions": s.skipped_encrypted,
            "sessions_without_payload": s.without_payload,
            "incomplete_directions": s.incomplete_directions,
            "truncated_units": s.truncated_units,
            "unparsed_tails": s.unparsed_tails,
            "units": len(self.units),
            "duplicate_units": self.duplicate_units,
            "duplicate_ratio": round(self.ratio, 6),
            "applications": self.applications,
            "expected_sizes": self.expected_sizes,
            "duplicates": self.duplicates,
            "failures": self.failures,
        }


def check(
    capture: Path,
    sessions: Sequence[Session],
    labels: Labels,
    max_duplicate_ratio: float | None = None,
) -> ContentResult:
    """Reassemble the payloads of a capture, count duplicates, and describe their sizes.

    ``max_duplicate_ratio`` overrides the value of the labels, which defaults to 1%.
    """
    settings = labels.content or ContentLabels()
    limit = settings.max_duplicate_ratio if max_duplicate_ratio is None else max_duplicate_ratio
    ignore = IGNORED_HEADERS | frozenset(settings.ignore_headers)
    units, stats = collect_units(sessions, read_flows(capture, sessions), labels, ignore)
    by_app: dict[str, list[Unit]] = defaultdict(list)
    for unit in units:
        by_app[unit.application].append(unit)
    failures: list[str] = []
    applications = []
    duplicates: list[dict[str, Any]] = []
    for application in sorted(by_app):
        app_units = by_app[application]
        directions: dict[str, Any] = {}
        app_duplicates = 0
        for direction in _DIRECTION_NAMES:
            group = [u for u in app_units if u.direction == direction]
            if group:
                directions[direction] = _group(group)
                app_duplicates += directions[direction]["duplicate_units"]
                duplicates += _repeated(application, direction, group)
        ratio = app_duplicates / len(app_units)
        entry: dict[str, Any] = {
            "application": application,
            "units": len(app_units),
            "distinct_units": len(app_units) - app_duplicates,
            "duplicate_units": app_duplicates,
            "duplicate_ratio": round(ratio, 6),
            "passed": ratio <= limit,
            "size": describe([u.size for u in app_units]),
            "directions": directions,
        }
        applications.append(entry)
    duplicates.sort(key=lambda d: (-d["copies"], -d["size"], d["sha256"]))
    duplicates = duplicates[:MAX_DUPLICATES_LISTED]
    for entry in applications:
        if not entry["passed"]:
            failures.append(_duplicate_failure(entry, limit, duplicates))
    expected = _compare_sizes(settings.sizes, by_app, failures)
    if not units:
        failures.append(
            "no payload could be analysed: "
            f"{stats.skipped_encrypted} sessions are encrypted, "
            f"{stats.without_payload} have no payload"
        )
    return ContentResult(
        max_duplicate_ratio=limit,
        ignored_headers=sorted(ignore),
        sessions_total=len(sessions),
        stats=stats,
        units=units,
        applications=applications,
        expected_sizes=expected,
        duplicates=duplicates,
        failures=failures,
    )


def _repeated(application: str, direction: str, group: Sequence[Unit]) -> list[dict[str, Any]]:
    seen: dict[str, list[Unit]] = defaultdict(list)
    for unit in group:
        seen[unit.digest].append(unit)
    return [
        {
            "application": application,
            "direction": direction,
            "copies": len(copies),
            "size": copies[0].size,
            "sha256": digest,
            "sessions": sorted({u.session for u in copies})[:5],
        }
        for digest, copies in seen.items()
        if len(copies) > 1
    ]


def _duplicate_failure(
    entry: Mapping[str, Any], limit: float, duplicates: Sequence[Mapping[str, Any]]
) -> str:
    text = (
        f"application {entry['application']}: {entry['duplicate_units']} of {entry['units']} "
        f"payloads ({entry['duplicate_ratio']:.2%}) repeat an earlier payload, above the "
        f"allowed {limit:.2%}"
    )
    top = next((d for d in duplicates if d["application"] == entry["application"]), None)
    if top is not None:
        text += (
            f"; the most repeated is a {top['direction']} payload of {top['size']} bytes "
            f"seen {top['copies']} times (sha256 {top['sha256'][:12]}) in sessions "
            f"{', '.join(str(s) for s in top['sessions'])}"
        )
    return text


def _within(observed: float, expected: float, tolerance: float) -> bool:
    if expected > 0:
        return abs(observed - expected) <= tolerance * expected
    return observed <= tolerance


def _compare_sizes(
    expectations: Mapping[str, SizeExpectation],
    by_app: Mapping[str, Sequence[Unit]],
    failures: list[str],
) -> list[dict[str, Any]]:
    out = []
    for application in sorted(expectations):
        want = expectations[application]
        group = [
            u
            for u in by_app.get(application, [])
            if want.direction == "both" or u.direction == want.direction
        ]
        if not group:
            failures.append(
                f"application {application}: no {want.direction} payload was observed to "
                "compare with the expected size distribution"
            )
            continue
        got = describe([u.size for u in group])
        median = got["quantiles"]["p50"]
        median_ok = _within(median, want.median, want.tolerance)
        cv_ok = want.cv is None or _within(got["cv"], want.cv, want.tolerance)
        if not median_ok:
            failures.append(
                f"application {application}: the median {want.direction} payload size is "
                f"{median:g} bytes, not within {want.tolerance:.0%} of the expected "
                f"{want.median:g} bytes"
            )
        if not cv_ok:
            failures.append(
                f"application {application}: the coefficient of variation of the "
                f"{want.direction} payload sizes is {got['cv']:g}, not within "
                f"{want.tolerance:.0%} of the expected {want.cv:g}"
            )
        out.append(
            {
                "application": application,
                "direction": want.direction,
                "units": got["count"],
                "tolerance": want.tolerance,
                "expected_median": want.median,
                "observed_median": median,
                "expected_cv": want.cv,
                "observed_cv": got["cv"],
                "passed": median_ok and cv_ok,
            }
        )
    return out
