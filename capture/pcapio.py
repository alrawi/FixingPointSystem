# SPDX-License-Identifier: GPL-3.0-only
"""Read classic pcap and pcapng captures with the standard library only.

Every byte of a capture is untrusted. A malformed header or block raises
:class:`CaptureError` with the file offset. A capture that ends inside a record, which
is what a killed ``tcpdump`` leaves behind, yields the frames before the cut and a
warning. Timestamps are integer nanoseconds since the Unix epoch.
"""

from __future__ import annotations

import mmap
import struct
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path

_PCAP_MAGIC = {
    b"\xd4\xc3\xb2\xa1": ("<", 1_000),
    b"\xa1\xb2\xc3\xd4": (">", 1_000),
    b"\x4d\x3c\xb2\xa1": ("<", 1),
    b"\xa1\xb2\x3c\x4d": (">", 1),
}
_PCAPNG_SHB = 0x0A0D0D0A
_PCAPNG_IDB = 0x00000001
_PCAPNG_SPB = 0x00000003
_PCAPNG_EPB = 0x00000006
_PCAPNG_BOM = 0x1A2B3C4D
_OPT_IF_NAME = 2
_OPT_IF_TSRESOL = 9
_OPT_IF_TSOFFSET = 14
_MAX_BLOCK = 256 * 1024 * 1024
_MAX_RECORD = 256 * 1024 * 1024
_NS = 1_000_000_000

Buffer = bytes | bytearray | memoryview | mmap.mmap


class CaptureError(Exception):
    """A capture file cannot be read. The message names the offset of the fault."""


@dataclass(frozen=True)
class Interface:
    """One capture interface. The id counts interfaces over the whole file."""

    id: int
    link_type: int
    snaplen: int
    name: str | None = None
    tsresol: str = "1e-6"
    section: int = 0


@dataclass(frozen=True)
class Frame:
    """One captured frame with its interface id, timestamp, and original length."""

    ts_ns: int
    interface: int
    link_type: int
    data: bytes
    orig_len: int


@dataclass
class Reader:
    """A lazy reader over one capture buffer.

    Call :meth:`frames` once. The attributes ``format``, ``interfaces``, and
    ``warnings`` are complete after the iteration ends.
    """

    buf: Buffer
    format: str = ""
    interfaces: list[Interface] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)

    def frames(self) -> Iterator[Frame]:
        """Yield every frame in file order."""
        size = len(self.buf)
        if size < 4:
            raise CaptureError(f"offset 0: file has {size} bytes, too short for a capture")
        head = bytes(self.buf[0:4])
        if head in _PCAP_MAGIC:
            self.format = "pcap"
            yield from self._pcap(*_PCAP_MAGIC[head])
        elif struct.unpack_from("<I", self.buf, 0)[0] == _PCAPNG_SHB:
            self.format = "pcapng"
            yield from self._pcapng()
        else:
            raise CaptureError(f"offset 0: unknown capture magic {head.hex()}")

    def _pcap(self, endian: str, unit_ns: int) -> Iterator[Frame]:
        buf = self.buf
        size = len(buf)
        if size < 24:
            raise CaptureError(f"offset 0: pcap header has {size} bytes, need 24")
        _, _, _, _, snaplen, network = struct.unpack_from(endian + "HHiIII", buf, 4)
        link_type = network & 0x0FFFFFFF
        self.interfaces.append(
            Interface(
                0,
                link_type,
                snaplen,
                tsresol="1e-9" if unit_ns == 1 else "1e-6",
            )
        )
        pos = 24
        while pos < size:
            if size - pos < 16:
                self.warnings.append(f"offset {pos}: capture ends inside a record header")
                return
            sec, frac, caplen, orig = struct.unpack_from(endian + "IIII", buf, pos)
            if caplen > _MAX_RECORD:
                raise CaptureError(f"offset {pos}: record length {caplen} is implausible")
            if size - pos - 16 < caplen:
                self.warnings.append(f"offset {pos}: capture ends inside a record")
                return
            data = bytes(buf[pos + 16 : pos + 16 + caplen])
            yield Frame(sec * _NS + frac * unit_ns, 0, link_type, data, orig)
            pos += 16 + caplen

    def _pcapng(self) -> Iterator[Frame]:
        buf = self.buf
        size = len(buf)
        pos = 0
        endian = "<"
        section = -1
        local: list[tuple[Interface, int, int]] = []  # interface, ticks per second, offset
        while pos < size:
            if size - pos < 12:
                self.warnings.append(f"offset {pos}: capture ends inside a block header")
                return
            if struct.unpack_from("<I", buf, pos)[0] == _PCAPNG_SHB:
                if size - pos < 12:
                    raise CaptureError(f"offset {pos}: section header block is cut off")
                bom = struct.unpack_from("<I", buf, pos + 8)[0]
                if bom == _PCAPNG_BOM:
                    endian = "<"
                elif struct.unpack_from(">I", buf, pos + 8)[0] == _PCAPNG_BOM:
                    endian = ">"
                else:
                    raise CaptureError(f"offset {pos}: bad pcapng byte order magic")
                section += 1
                local = []
            btype, total = struct.unpack_from(endian + "II", buf, pos)
            if total < 12 or total % 4 or total > _MAX_BLOCK:
                raise CaptureError(f"offset {pos}: bad block length {total}")
            if pos + total > size:
                self.warnings.append(f"offset {pos}: capture ends inside a block")
                return
            if struct.unpack_from(endian + "I", buf, pos + total - 4)[0] != total:
                raise CaptureError(f"offset {pos}: block trailer length differs from header")
            body = bytes(buf[pos + 8 : pos + total - 4])
            if btype == _PCAPNG_IDB:
                local.append(self._idb(body, pos, endian, section))
            elif btype == _PCAPNG_EPB:
                frame = self._epb(body, pos, endian, local)
                if frame is not None:
                    yield frame
            elif btype == _PCAPNG_SPB:
                frame = self._spb(body, pos, endian, local)
                if frame is not None:
                    yield frame
            pos += total

    def _idb(self, body: bytes, pos: int, endian: str, section: int) -> tuple[Interface, int, int]:
        if len(body) < 8:
            raise CaptureError(f"offset {pos}: interface description block is too short")
        link_type, _, snaplen = struct.unpack_from(endian + "HHI", body, 0)
        name: str | None = None
        ticks = 1_000_000
        resol = "1e-6"
        offset = 0
        for code, value in _options(body[8:], endian):
            if code == _OPT_IF_NAME:
                name = value.decode("utf-8", "replace")
            elif code == _OPT_IF_TSRESOL and len(value) == 1:
                byte = value[0]
                exp = byte & 0x7F
                if exp > 60:
                    raise CaptureError(f"offset {pos}: implausible if_tsresol {byte:#x}")
                if byte & 0x80:
                    ticks = 1 << exp
                    resol = f"2^-{exp}"
                else:
                    ticks = 10**exp
                    resol = f"1e-{exp}"
            elif code == _OPT_IF_TSOFFSET and len(value) == 8:
                offset = struct.unpack_from(endian + "q", value, 0)[0]
        info = Interface(len(self.interfaces), link_type, snaplen, name, resol, section)
        self.interfaces.append(info)
        return info, ticks, offset

    def _epb(
        self,
        body: bytes,
        pos: int,
        endian: str,
        local: list[tuple[Interface, int, int]],
    ) -> Frame | None:
        if len(body) < 20:
            raise CaptureError(f"offset {pos}: enhanced packet block is too short")
        index, hi, lo, caplen, orig = struct.unpack_from(endian + "IIIII", body, 0)
        if index >= len(local):
            self.warnings.append(f"offset {pos}: packet names unknown interface {index}")
            return None
        if caplen > len(body) - 20:
            raise CaptureError(f"offset {pos}: captured length {caplen} exceeds the block")
        info, ticks, offset = local[index]
        ticks_total = (hi << 32) | lo
        ts_ns = (ticks_total * _NS) // ticks + offset * _NS
        return Frame(ts_ns, info.id, info.link_type, body[20 : 20 + caplen], orig)

    def _spb(
        self,
        body: bytes,
        pos: int,
        endian: str,
        local: list[tuple[Interface, int, int]],
    ) -> Frame | None:
        if len(body) < 4 or not local:
            raise CaptureError(f"offset {pos}: simple packet block without an interface")
        orig = struct.unpack_from(endian + "I", body, 0)[0]
        info = local[0][0]
        data = body[4 : 4 + min(orig, len(body) - 4, info.snaplen or orig)]
        note = "simple packet blocks carry no timestamp, so they count as time 0"
        if note not in self.warnings:
            self.warnings.append(note)
        return Frame(0, info.id, info.link_type, data, orig)


def _options(raw: bytes, endian: str) -> Iterator[tuple[int, bytes]]:
    """Yield the (code, value) options of a block, and stop at a malformed one."""
    pos = 0
    while len(raw) - pos >= 4:
        code, length = struct.unpack_from(endian + "HH", raw, pos)
        if code == 0:
            return
        value = raw[pos + 4 : pos + 4 + length]
        if len(value) < length:
            return
        yield code, value
        pos += 4 + ((length + 3) & ~3)


def open_capture(path: Path) -> tuple[Reader, mmap.mmap | None]:
    """Map a capture file and return its reader and the mapping to close afterwards."""
    try:
        with path.open("rb") as handle:
            if path.stat().st_size == 0:
                raise CaptureError(f"{path}: file is empty")
            mapping = mmap.mmap(handle.fileno(), 0, access=mmap.ACCESS_READ)
    except OSError as exc:
        raise CaptureError(f"{path}: cannot read capture: {exc}") from exc
    return Reader(mapping), mapping
