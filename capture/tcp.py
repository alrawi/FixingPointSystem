# SPDX-License-Identifier: GPL-3.0-only
"""Reassemble one direction of a TCP connection.

A stream tracks offsets relative to its anchor, which is the sequence number after the
SYN or, when the capture missed the SYN, the first data segment seen. Offsets are
unwrapped past 32 bits. The first copy of every byte wins, and a later copy that
differs is counted as an overlap mismatch.
"""

from __future__ import annotations

from bisect import bisect_right
from dataclasses import dataclass, field

TAIL_WINDOW = 64 * 1024
MAX_BUFFERED = 16 * 1024 * 1024
_HALF = 1 << 31
_MOD = 1 << 32


@dataclass(slots=True)
class Delivery:
    """Bytes that became available in stream order at one time."""

    ts_ns: int
    length: int
    data: bytes | None
    gap_before: int = 0


@dataclass
class Stream:
    """One direction of a TCP connection."""

    keep_payload: bool = False
    base_seq: int | None = None
    syn: bool = False
    fin_ts: int | None = None
    rst_ts: int | None = None
    next_off: int = 0
    low: int = 0
    _first: int = 0
    highest: int = 0
    unique_bytes: int = 0
    retrans_packets: int = 0
    retrans_bytes: int = 0
    out_of_order_packets: int = 0
    mismatch_packets: int = 0
    skipped_bytes: int = 0
    overflowed: bool = False
    deliveries: list[Delivery] = field(default_factory=list)
    _ref_seq: int = 0
    _ref_off: int = 0
    _starts: list[int] = field(default_factory=list)
    _held: list[tuple[int, int, bytes, int]] = field(default_factory=list)
    _early: list[tuple[int, int]] = field(default_factory=list)
    _held_bytes: int = 0
    _tail: bytearray = field(default_factory=bytearray)
    _tail_start: int = 0

    def anchor(self, seq: int, *, syn: bool) -> None:
        """Fix offset 0 at ``seq``. A SYN anchors at the next sequence number."""
        if self.base_seq is not None:
            return
        self.base_seq = (seq + 1) % _MOD if syn else seq % _MOD
        self.syn = syn
        self._ref_seq = self.base_seq
        self._ref_off = 0

    def offset(self, seq: int) -> int:
        """Unwrap a 32 bit sequence number to a signed offset from the anchor."""
        delta = ((seq - self._ref_seq + _HALF) % _MOD) - _HALF
        off = self._ref_off + delta
        if off > self._ref_off:
            self._ref_seq, self._ref_off = seq, off
        return off

    def add(self, ts_ns: int, seq: int, wire_len: int, data: bytes) -> None:
        """Add a segment of ``wire_len`` bytes whose first ``len(data)`` bytes were captured."""
        if wire_len <= 0:
            return
        first = self.base_seq is None
        if first:
            self.anchor(seq, syn=False)
            start = 0
            self.next_off = self.low = self.highest = 0
        else:
            start = self.offset(seq)
        if self.syn and start < 0:  # before the first byte after the SYN
            clip = -start
            if clip >= wire_len:
                self.retrans_packets += 1
                self.retrans_bytes += wire_len
                return
            start, wire_len, data = 0, wire_len - clip, data[clip:]
        if len(data) < wire_len:
            data = data + bytes(wire_len - len(data))
        end = start + wire_len
        if not self.syn and start < self.low:
            self.low = start
        covered = self._covered(start, end)
        dup = sum(b - a for a, b in covered)
        if dup:
            self.retrans_packets += 1
            self.retrans_bytes += dup
            if self._mismatch(start, data, covered):
                self.mismatch_packets += 1
        elif start < self.highest:
            self.out_of_order_packets += 1
        for a, b in _subtract(start, end, covered):
            self._store(ts_ns, a, data[a - start : b - start])
        self.highest = max(self.highest, end)
        self._advance(ts_ns)
        if self._held_bytes > MAX_BUFFERED:
            self.overflowed = True
            self._flush()

    def finish(self) -> None:
        """Deliver bytes that wait behind a hole, and record the skipped bytes."""
        if self._held:
            self._flush()

    @property
    def missing_bytes(self) -> int:
        """Bytes between the first and last offset that the capture never held."""
        if self.base_seq is None:
            return 0
        return max(0, self.highest - self.low - self.unique_bytes)

    def _covered(self, start: int, end: int) -> list[tuple[int, int]]:
        spans: list[tuple[int, int]] = []
        a, b = max(start, self._first), min(end, self.next_off)
        if a < b:
            spans.append((a, b))
        for s, e in self._early:
            a, b = max(start, s), min(end, e)
            if a < b:
                spans.append((a, b))
        i = max(0, bisect_right(self._starts, start) - 1)
        for s, e, _, _ in self._held[i:]:
            if s >= end:
                break
            a, b = max(start, s), min(end, e)
            if a < b:
                spans.append((a, b))
        return spans

    def _mismatch(self, start: int, data: bytes, covered: list[tuple[int, int]]) -> bool:
        for a, b in covered:
            new = data[a - start : b - start]
            if a >= self._tail_start and b <= self.next_off:
                old = bytes(self._tail[a - self._tail_start : b - self._tail_start])
            else:
                old = b""
                for s, e, held, _ in self._held:
                    if s <= a and b <= e:
                        old = held[a - s : b - s]
                        break
            if old and old != new:
                return True
        return False

    def _store(self, ts_ns: int, start: int, data: bytes) -> None:
        self.unique_bytes += len(data)
        i = bisect_right(self._starts, start)
        self._starts.insert(i, start)
        self._held.insert(i, (start, start + len(data), data, ts_ns))
        self._held_bytes += len(data)

    def _advance(self, ts_ns: int) -> None:
        while self._held and self._held[0][0] <= self.next_off:
            start, end, data, _ = self._held.pop(0)
            self._starts.pop(0)
            self._held_bytes -= len(data)
            if end <= self.next_off:  # late bytes from before the first segment seen
                self._early.append((start, end))
                self.deliveries.append(
                    Delivery(ts_ns, len(data), data if self.keep_payload else None)
                )
                continue
            self._deliver(ts_ns, data[self.next_off - start :], 0)

    def _flush(self) -> None:
        while self._held:
            start, _, data, ts_ns = self._held.pop(0)
            self._starts.pop(0)
            self._held_bytes -= len(data)
            gap = max(0, start - self.next_off)
            self.skipped_bytes += gap
            self._deliver(ts_ns, data, gap)

    def _deliver(self, ts_ns: int, data: bytes, gap: int) -> None:
        if self.deliveries:
            ts_ns = max(ts_ns, self.deliveries[-1].ts_ns)
        self.deliveries.append(Delivery(ts_ns, len(data), data if self.keep_payload else None, gap))
        if gap:
            self._tail.clear()
            self.next_off += gap
            self._tail_start = self.next_off
        self._tail += data
        self.next_off += len(data)
        if len(self._tail) > TAIL_WINDOW:
            drop = len(self._tail) - TAIL_WINDOW
            del self._tail[:drop]
            self._tail_start += drop


def _subtract(start: int, end: int, covered: list[tuple[int, int]]) -> list[tuple[int, int]]:
    """Return the parts of [start, end) outside the sorted covered spans."""
    parts: list[tuple[int, int]] = []
    pos = start
    for a, b in sorted(covered):
        if a > pos:
            parts.append((pos, a))
        pos = max(pos, b)
    if pos < end:
        parts.append((pos, end))
    return parts
