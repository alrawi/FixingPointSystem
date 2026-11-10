# SPDX-License-Identifier: GPL-3.0-only
"""Framing and dynamic field detection for scenario generation (CAP-02).

The module works on the payload of one *unit*: a TCP turn or a UDP datagram. It finds

* framing, which tells the receiving side where records end (HTTP messages with a
  ``Content-Length``, binary length prefixed frames) and which payload regions the
  generator may replace by ``fill(n)``;
* dynamic fields: lengths, HTTP dates, epoch timestamps, cookies, and identifiers.

Every heuristic has a guard that keeps it from rewriting ordinary content. The wiki page
``docs/wiki/Capture.md`` lists each heuristic with its guard, and the tests exercise the
guards with data that must stay unchanged.
"""

from __future__ import annotations

import math
import re
from collections import Counter
from collections.abc import Iterable
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Literal

from .templates import escape_literal


@dataclass
class Config:
    """Tuning of the generator. The defaults are documented in the wiki page."""

    fill_min: int = 256  # payload regions at least this long may become fill(n)
    opaque_min: int = 2048  # unframed binary units at least this long become fill(n)
    timestamp_window_s: int = 86400  # a timestamp within this of the capture time is dynamic
    id_min_len: int = 8
    keep_timing: bool = False
    min_delay_ms: int = 10
    detect_fields: bool = True


@dataclass
class Span:
    """A byte range of a unit that the template replaces by ``tmpl``."""

    start: int
    end: int
    tmpl: str
    kind: str


@dataclass
class Record:
    """A byte range that the receiving side consumes with one ``expect``."""

    start: int
    end: int
    kind: str  # head, body, frame
    length_var: str | None = None


@dataclass
class Analysis:
    """Framing of one unit."""

    protocol: str = "raw"  # http, frames, opaque, raw
    records: list[Record] = field(default_factory=list)
    spans: list[Span] = field(default_factory=list)


class VarAlloc:
    """Scenario variables with unique names, in declaration order."""

    def __init__(self) -> None:
        self.values: dict[str, int | str] = {}
        self._counters: Counter[str] = Counter()
        self._lengths: dict[tuple[str, int], str] = {}

    def unique(self, base: str) -> str:
        base = re.sub(r"[^a-z0-9_]+", "_", base.lower()).strip("_") or "v"
        if not base[0].isalpha():
            base = "v_" + base
        base = base[:50]
        self._counters[base] += 1
        n = self._counters[base]
        name = base if n == 1 and base not in self.values else f"{base}_{n}"
        while name in self.values:
            self._counters[base] += 1
            name = f"{base}_{self._counters[base]}"
        return name

    def declare(self, name: str, value: int | str) -> None:
        self.values[name] = value

    def length(self, direction: str, n: int) -> str:
        """The variable for a body size in one direction, shared by equal sizes."""
        key = (direction, n)
        if key not in self._lengths:
            name = self.unique(f"{direction}_len")
            self._lengths[key] = name
            self.values[name] = n
        return self._lengths[key]


# -- framing ------------------------------------------------------------------------

_START = re.compile(
    rb"(?:([A-Z]{3,10}) [^\r\n ]+ HTTP/1\.[01]|HTTP/1\.[01] [0-9]{3}(?: [^\r\n]*)?)\r\n"
)
_HEADER = re.compile(rb"\r\n(content-length|transfer-encoding):[ \t]*([^\r\n]*)", re.I)


def _printable_share(data: bytes) -> float:
    if not data:
        return 1.0
    return sum(1 for b in data if 32 <= b < 127 or b in (9, 10, 13)) / len(data)


def _http(data: bytes, direction: str, cfg: Config, alloc: VarAlloc) -> Analysis | None:
    """Split a unit into HTTP/1.x messages.

    Guard: a message counts only when its start line is a valid request or status line,
    its head ends inside the unit, and a ``Content-Length`` is unique, numeric, and
    covered by the bytes that follow. A message that fails the guard keeps its length
    literal, and parsing stops there.
    """
    out = Analysis("http")
    pos = 0
    while pos < len(data):
        m = _START.match(data, pos)
        end = data.find(b"\r\n\r\n", pos)
        if m is None or end < 0:
            break
        head_end = end + 4
        is_request = m.group(1) is not None
        headers = list(_HEADER.finditer(data, pos, head_end))
        clen = [h for h in headers if h.group(1).lower() == b"content-length"]
        chunked = any(h.group(1).lower() == b"transfer-encoding" for h in headers)
        out.records.append(Record(pos, head_end, "head"))
        if chunked or len(clen) > 1:
            pos = head_end
            break  # the body is not delimited by one length, so the rest stays raw
        if not clen:
            body_end = head_end if is_request else len(data)
            if body_end > head_end:
                out.records.append(Record(head_end, body_end, "body"))
            pos = body_end
            continue
        digits = clen[0].group(2).rstrip(b" \t")
        if not re.fullmatch(rb"[0-9]{1,12}", digits):
            pos = head_end
            break
        n = int(digits)
        if head_end + n > len(data):  # the body is cut off or continues in a later unit
            out.records.append(Record(head_end, len(data), "body"))
            pos = len(data)
            break
        var = None
        if n >= cfg.fill_min:
            var = alloc.length(direction, n)
            start = clen[0].start(2)
            out.spans.append(Span(start, start + len(digits), f"{{{{ vars.{var} }}}}", "length"))
            out.spans.append(Span(head_end, head_end + n, f"{{{{ fill(vars.{var}) }}}}", "body"))
        if n:
            out.records.append(Record(head_end, head_end + n, "body", var))
        pos = head_end + n
    if not out.records:
        return None
    if pos < len(data):
        out.records.append(Record(pos, len(data), "raw"))
    return out


# (offset of the length field, width in bytes). The offset 3 with width 2 is the TLS record
# header, and the others are the usual length first layouts.
_FRAME_FORMS = ((0, 1), (0, 2), (0, 4), (2, 2), (3, 2))


def _frames(data: bytes, cfg: Config) -> Analysis | None:
    """Split a unit into binary frames that carry their own length.

    A frame has a header of ``offset + width`` bytes whose last ``width`` bytes hold the
    payload length (big or little endian) or the whole frame length. Guard: the frames
    must tile the unit exactly, from the first byte to the last, each with a payload, and
    the length bytes of the first frame must hold a byte outside printable ASCII, so that
    text which happens to add up never matches.
    """
    n = len(data)
    if n < 4:
        return None
    for offset, width in _FRAME_FORMS:
        orders: tuple[Literal["big", "little"], ...] = ("big", "little") if width > 1 else ("big",)
        for order in orders:
            for bias in (offset + width, 0):
                frames = _tile(data, offset, width, order, bias)
                if frames is None:
                    continue
                first = data[offset : offset + width]
                if all(32 <= b < 127 for b in first):
                    continue
                out = Analysis("frames")
                for start, end, hdr in frames:
                    out.records.append(Record(start, end, "frame"))
                    if end - hdr >= cfg.fill_min:
                        out.spans.append(Span(hdr, end, f"{{{{ fill({end - hdr}) }}}}", "payload"))
                return out
    return None


def _tile(
    data: bytes, offset: int, width: int, order: Literal["big", "little"], bias: int
) -> list[tuple[int, int, int]] | None:
    pos, frames = 0, []
    hdr = offset + width
    while pos < len(data):
        if pos + hdr > len(data):
            return None
        size = int.from_bytes(data[pos + offset : pos + hdr], order) + bias
        if size <= hdr or pos + size > len(data):
            return None
        frames.append((pos, pos + size, pos + hdr))
        pos += size
    return frames or None


def analyze_framing(
    data: bytes, direction: str, cfg: Config, alloc: VarAlloc, *, datagram: bool = False
) -> Analysis:
    """Return the framing of one unit and the payload regions that become ``fill(n)``.

    A datagram skips the stream framing rules, because each datagram is one unit already.
    """
    if cfg.detect_fields:
        found = None if datagram else _http(data, direction, cfg, alloc) or _frames(data, cfg)
        if found is not None:
            return found
        if len(data) >= cfg.opaque_min and _printable_share(data) < 0.85:
            # Guard: only binary data is replaced; text stays as it was captured.
            return Analysis(
                "opaque", [], [Span(0, len(data), f"{{{{ fill({len(data)}) }}}}", "opaque")]
            )
    return Analysis()


# -- fields -------------------------------------------------------------------------


def entropy(text: str) -> float:
    """Shannon entropy of the characters of ``text`` in bits per character."""
    if not text:
        return 0.0
    counts = Counter(text)
    n = len(text)
    return -sum(c / n * math.log2(c / n) for c in counts.values())


_DIGITS = "0123456789"


def _digit_chunks(n: int) -> list[str]:
    """Placeholders for ``n`` random digits with no leading zero in any chunk."""
    out = []
    while n > 0:
        w = min(n, 18)
        out.append(f"{{{{ rand_int({10 ** (w - 1)}, {10**w - 1}) }}}}")
        n -= w
    return out


def id_template(value: str) -> str:
    """Template text of the same shape as ``value`` with random digits in place of letters
    and digits, and with its separators kept.

    ``rand_int`` is the only random source that makes visible text, so a token of hex or
    base64 characters becomes a token of digits. The length and the separators stay. A
    result that the ``set`` action would read as a number gets one literal letter in front,
    so that ``set`` keeps it as text: ``set`` parses integers and floats, and it would
    cut a trailing zero or switch to a float after 18 digits.
    """
    parts = re.findall(r"[A-Za-z0-9]+|[^A-Za-z0-9]", value)
    separators = [p for p in parts if not p[0].isalnum()]
    numeric = (
        all(p == "." for p in separators)
        and len(separators) <= 1
        and parts[0][0].isalnum()
        and parts[-1][0].isalnum()
    )
    exact = not separators and len(value) <= 18
    out: list[str] = []
    for i, part in enumerate(parts):
        if not part[0].isalnum():
            out.append(escape_literal(part.encode("ascii", "replace")))
            continue
        run = len(part)
        if i == 0 and numeric and not exact:
            out.append("x")
            run -= 1
        out.extend(_digit_chunks(run))
    return "".join(out)


def epoch_template(value: int, digits: int, window: int) -> str:
    """A random epoch value within ``window`` of ``value`` that keeps the digit count."""
    lo = max(10 ** (digits - 1), value - window)
    hi = min(10**digits - 1, value + window)
    return f"{{{{ rand_int({lo}, {hi}) }}}}"


_MONTHS = ("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec")
_DAYS = ("Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun")
HTTP_DATE = re.compile(
    rb"(?<![A-Za-z0-9])(Mon|Tue|Wed|Thu|Fri|Sat|Sun), ([0-9]{2}) "
    rb"(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) ([0-9]{4}) "
    rb"([0-9]{2}):([0-9]{2}):([0-9]{2}) GMT(?![A-Za-z0-9])"
)


def parse_http_date(m: re.Match[bytes]) -> float | None:
    """Epoch seconds of an IMF-fixdate match, or None when the date does not exist or
    its weekday is wrong. The weekday check is the guard against look-alike text."""
    day, month = int(m.group(2)), _MONTHS.index(m.group(3).decode()) + 1
    try:
        when = datetime(
            int(m.group(4)),
            month,
            day,
            int(m.group(5)),
            int(m.group(6)),
            int(m.group(7)),
            tzinfo=timezone.utc,
        )
    except ValueError:
        return None
    if _DAYS[when.weekday()] != m.group(1).decode():
        return None
    return when.timestamp()


EPOCH = re.compile(rb"(?<![A-Za-z0-9.+\-])([0-9]{10}|[0-9]{13})(?![A-Za-z0-9]|\.[0-9])")
_COOKIE_NAME = rb"[!#$%&'*+.^_`|~0-9A-Za-z\-]+"
SET_COOKIE = re.compile(rb"(?im)^set-cookie:[ \t]*(" + _COOKIE_NAME + rb")=([^;\r\n]*)")
COOKIE_HEADER = re.compile(rb"(?im)^cookie:[ \t]*([^\r\n]*)")
COOKIE_PAIR = re.compile(rb"(" + _COOKIE_NAME + rb")=([^;\r\n]*)")
UUID = re.compile(
    rb"(?<![0-9A-Za-z\-])[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-"
    rb"[0-9a-fA-F]{12}(?![0-9A-Za-z\-])"
)
TOKEN = re.compile(rb"(?<![A-Za-z0-9_\-.@])[A-Za-z0-9][A-Za-z0-9_\-]{7,127}(?![A-Za-z0-9_\-.@])")
_REQUEST_LINE = re.compile(rb"^[A-Z]{3,10} [^\r\n ]+ HTTP/1\.[01]\r\n")


def random_looking(token: str, *, strong: bool) -> bool:
    """Whether ``token`` looks like a random identifier.

    Guard against words, versions, and dates: it needs a letter and a digit (or at least
    twelve digits), six distinct characters, no run of four equal characters, and a
    character entropy of at least 3.0 bits. ``strong`` raises the bar for a token that
    appears only once, to 16 characters and 3.3 bits, because a single occurrence has no
    request/response echo to confirm it.
    """
    digits = sum(c.isdigit() for c in token)
    letters = sum(c.isalpha() for c in token)
    if digits == len(token.replace("-", "").replace("_", "")):
        if len(token) < 12:
            return False
    elif not (digits and letters):
        return False
    if len(set(token)) < 6 or re.search(r"(.)\1{3}", token):
        return False
    if strong and len(token) < 16:
        return False
    return entropy(token) >= (3.3 if strong else 3.0)


def cookie_like(value: str) -> bool:
    """Guard for cookies: at least six characters with an entropy of 2.5 bits, so that
    ``lang=en`` and ``theme=dark`` stay as they are."""
    return len(value) >= 6 and entropy(value) >= 2.5 and not value.isalpha()


@dataclass
class Occurrence:
    unit: int
    start: int
    end: int


@dataclass
class Field:
    """One dynamic value and every place where it occurs."""

    kind: str  # cookie, id, epoch_s, epoch_ms, http_date
    name: str
    value: str
    occurrences: list[Occurrence]
    gen: str = ""  # template text that produces a fresh value of the same shape

    @property
    def units(self) -> list[int]:
        return sorted({o.unit for o in self.occurrences})


class _Claims:
    """Byte ranges of each unit that a span or a field already owns."""

    def __init__(self, analyses: list[Analysis]) -> None:
        self.ranges: list[list[tuple[int, int]]] = [
            [(s.start, s.end) for s in a.spans] for a in analyses
        ]

    def free(self, unit: int, start: int, end: int) -> bool:
        return all(end <= s or start >= e for s, e in self.ranges[unit])

    def take(self, unit: int, start: int, end: int) -> None:
        self.ranges[unit].append((start, end))

    def segments(self, unit: int, size: int) -> Iterable[tuple[int, int]]:
        """The ranges of the unit that no span replaces, in order."""
        pos = 0
        for s, e in sorted(self.ranges[unit]):
            if s > pos:
                yield pos, s
            pos = max(pos, e)
        if pos < size:
            yield pos, size


def detect_fields(
    units: list[tuple[str, bytes]],
    analyses: list[Analysis],
    ref_times: list[float],
    cfg: Config,
    alloc: VarAlloc,
) -> list[Field]:
    """Find dynamic fields in the units of one flow.

    ``units`` lists ``(direction, payload)`` with the direction ``up`` for client to
    server, and ``ref_times`` the capture time of each unit in epoch seconds. The
    heuristics run in this order, and a range that an earlier one claimed stays with it:
    cookies, HTTP dates, epoch timestamps, identifiers.
    """
    claims = _Claims(analyses)
    found: list[Field] = []
    found += _cookies(units, claims, alloc)
    found += _http_dates(units, claims, ref_times, cfg, alloc)
    found += _epochs(units, claims, ref_times, cfg, alloc)
    found += _identifiers(units, claims, cfg, alloc)
    return found


def _cookies(units: list[tuple[str, bytes]], claims: _Claims, alloc: VarAlloc) -> list[Field]:
    """Cookies: a Set-Cookie value that a later Cookie header sends back.

    Guard: the value must come back unchanged in a later unit, and it must pass
    ``cookie_like``. A Set-Cookie that nobody echoes is left to the identifier rule.
    """
    sets: dict[tuple[bytes, bytes], list[Occurrence]] = {}
    echoes: dict[tuple[bytes, bytes], list[Occurrence]] = {}
    for u, (_, data) in enumerate(units):
        for m in SET_COOKIE.finditer(data):
            if claims.free(u, m.start(2), m.end(2)) and m.end(2) > m.start(2):
                sets.setdefault((m.group(1), m.group(2)), []).append(
                    Occurrence(u, m.start(2), m.end(2))
                )
        for h in COOKIE_HEADER.finditer(data):
            for p in COOKIE_PAIR.finditer(h.group(1)):
                s, e = h.start(1) + p.start(2), h.start(1) + p.end(2)
                if e > s and claims.free(u, s, e):
                    echoes.setdefault((p.group(1), p.group(2)), []).append(Occurrence(u, s, e))
    out: list[Field] = []
    for key, first in sorted(sets.items(), key=lambda kv: kv[1][0].unit):
        later = [o for o in echoes.get(key, []) if o.unit > first[0].unit]
        value = key[1].decode("latin-1")
        if not later or not cookie_like(value):
            continue
        occ = sorted(first + later, key=lambda o: (o.unit, o.start))
        for o in occ:
            claims.take(o.unit, o.start, o.end)
        name = alloc.unique("cookie_" + key[0].decode("latin-1"))
        out.append(Field("cookie", name, value, occ, id_template(value)))
    return out


def _http_dates(
    units: list[tuple[str, bytes]], claims: _Claims, refs: list[float], cfg: Config, alloc: VarAlloc
) -> list[Field]:
    """HTTP dates close to the capture time.

    Guard: the weekday must match the date and the date must lie within the timestamp
    window of the capture time. A ``Last-Modified`` of last year is not dynamic and stays.
    The template has no clock function, so the value becomes a scenario variable that
    keeps the observed date, which both sides read.
    """
    by_value: dict[str, list[Occurrence]] = {}
    for u, (_, data) in enumerate(units):
        for m in HTTP_DATE.finditer(data):
            when = parse_http_date(m)
            if when is None or abs(when - refs[u]) > cfg.timestamp_window_s:
                continue
            if claims.free(u, m.start(), m.end()):
                by_value.setdefault(m.group(0).decode("ascii"), []).append(
                    Occurrence(u, m.start(), m.end())
                )
    out = []
    for value, occ in by_value.items():
        for o in occ:
            claims.take(o.unit, o.start, o.end)
        out.append(Field("http_date", alloc.unique("http_date"), value, occ))
    return out


def _epochs(
    units: list[tuple[str, bytes]], claims: _Claims, refs: list[float], cfg: Config, alloc: VarAlloc
) -> list[Field]:
    """Epoch seconds (10 digits) and milliseconds (13 digits) close to the capture time.

    Guard: the number must lie within the timestamp window of the capture time, and it
    must stand alone, with no letter, digit, sign, or decimal part next to it. Other
    ten digit numbers, such as identifiers and phone numbers, are far from the capture
    time with overwhelming probability.
    """
    by_value: dict[str, list[Occurrence]] = {}
    for u, (_, data) in enumerate(units):
        for m in EPOCH.finditer(data):
            digits = m.group(1)
            seconds = int(digits) / (1000 if len(digits) == 13 else 1)
            if abs(seconds - refs[u]) > cfg.timestamp_window_s:
                continue
            if claims.free(u, m.start(1), m.end(1)):
                by_value.setdefault(digits.decode(), []).append(Occurrence(u, m.start(1), m.end(1)))
    out = []
    for value, occ in by_value.items():
        for o in occ:
            claims.take(o.unit, o.start, o.end)
        ms = len(value) == 13
        window = cfg.timestamp_window_s * (1000 if ms else 1)
        out.append(
            Field(
                "epoch_ms" if ms else "epoch_s",
                alloc.unique("ts_ms" if ms else "ts"),
                value,
                occ,
                epoch_template(int(value), len(value), window),
            )
        )
    return out


def _identifiers(
    units: list[tuple[str, bytes]], claims: _Claims, cfg: Config, alloc: VarAlloc
) -> list[Field]:
    """UUIDs and random looking tokens.

    A UUID always counts. Any other token must pass ``random_looking``, and a token that
    occurs in one unit only must pass its strong form. A token in the request target of
    an HTTP request is a resource name, so it counts only when it also appears in another
    unit (a redirect or an echo). Tokens next to a dot or an at sign are host names and
    addresses, which the pattern skips.
    """
    cands: dict[str, list[Occurrence]] = {}
    in_target: set[tuple[int, int]] = set()
    for u, (_, data) in enumerate(units):
        request = _REQUEST_LINE.match(data)
        line_end = request.end() if request else 0
        for pattern, is_uuid in ((UUID, True), (TOKEN, False)):
            for seg_start, seg_end in list(claims.segments(u, len(data))):
                for m in pattern.finditer(data, seg_start, seg_end):
                    if not claims.free(u, m.start(), m.end()):
                        continue
                    text = m.group(0).decode("ascii")
                    if not is_uuid and UUID.fullmatch(m.group(0)):
                        continue
                    cands.setdefault(text, []).append(Occurrence(u, m.start(), m.end()))
                    if request and m.start() < line_end:
                        in_target.add((u, m.start()))
    out = []
    for text, occ in cands.items():
        is_uuid = bool(UUID.fullmatch(text.encode()))
        spread = len({o.unit for o in occ}) > 1
        if not is_uuid:
            if len(text) < cfg.id_min_len or not random_looking(text, strong=not spread):
                continue
            if not spread and all((o.unit, o.start) in in_target for o in occ):
                continue
        occ = [o for o in occ if claims.free(o.unit, o.start, o.end)]
        if not occ:
            continue
        occ.sort(key=lambda o: (o.unit, o.start))
        for o in occ:
            claims.take(o.unit, o.start, o.end)
        out.append(Field("id", alloc.unique("id"), text, occ, id_template(text)))
    return out
