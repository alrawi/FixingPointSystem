# SPDX-License-Identifier: GPL-3.0-only
"""Read pcap and pcapng captures and group their packets into sessions.

The reader uses only the standard library and treats every byte of the capture as
untrusted. Malformed records raise :class:`CaptureError`, and malformed or
unsupported packets inside well formed records are counted and skipped.
"""

from __future__ import annotations

import ipaddress
import struct
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path

# Link types from https://www.tcpdump.org/linktypes.html that the reader decodes.
LINKTYPE_NULL = 0
LINKTYPE_ETHERNET = 1
LINKTYPE_RAW = 101
LINKTYPE_LINUX_SLL = 113
LINKTYPE_IPV4 = 228
LINKTYPE_IPV6 = 229

_PCAP_MAGIC = {
    b"\xd4\xc3\xb2\xa1": ("<", 1e-6),
    b"\xa1\xb2\xc3\xd4": (">", 1e-6),
    b"\x4d\x3c\xb2\xa1": ("<", 1e-9),
    b"\xa1\xb2\x3c\x4d": (">", 1e-9),
}
_PCAPNG_SHB = 0x0A0D0D0A
_PCAPNG_IDB = 0x00000001
_PCAPNG_SPB = 0x00000003
_PCAPNG_EPB = 0x00000006
_PCAPNG_BOM = 0x1A2B3C4D
_MAX_BLOCK = 64 * 1024 * 1024

_ETH_IPV4 = 0x0800
_ETH_IPV6 = 0x86DD
_ETH_VLAN = (0x8100, 0x88A8, 0x9100)
_IP_TCP = 6
_IP_UDP = 17
_IPV6_EXTENSION_HEADERS = (0, 43, 60)
_TCP_SYN = 0x02
_TCP_ACK = 0x10


class CaptureError(Exception):
    """A capture file cannot be read, with the file and the offset of the fault."""


@dataclass(frozen=True)
class Frame:
    """One captured frame with its link type and timestamp in seconds."""

    timestamp: float
    link_type: int
    data: bytes


@dataclass(frozen=True)
class Packet:
    """The transport view of one decoded IPv4 or IPv6 packet."""

    timestamp: float
    transport: str
    src_ip: str
    src_port: int
    dst_ip: str
    dst_port: int
    payload_len: int
    tcp_flags: int = 0


@dataclass(frozen=True, order=True)
class SessionKey:
    """A direction independent key for a transport session.

    The two endpoints are stored in sorted order, so both directions of a session
    map to the same key. Classifier output joins the session table through it.
    """

    transport: str
    a_ip: str
    a_port: int
    b_ip: str
    b_port: int

    @classmethod
    def of(cls, transport: str, ip1: str, port1: int, ip2: str, port2: int) -> SessionKey:
        """Build the key of the session between two endpoints in either order."""
        first = (_ip_sort_key(ip1), port1, ip1)
        second = (_ip_sort_key(ip2), port2, ip2)
        if first <= second:
            return cls(transport, ip1, port1, ip2, port2)
        return cls(transport, ip2, port2, ip1, port1)


@dataclass
class Session:
    """A bidirectional TCP or UDP session observed in a capture.

    The client is the sender of the first SYN without ACK for TCP, and the sender
    of the first packet otherwise.
    """

    id: int
    transport: str
    client_ip: str
    client_port: int
    server_ip: str
    server_port: int
    first_seen: float
    last_seen: float
    packets: int = 0
    payload_bytes: int = 0
    client_known: bool = False

    @property
    def key(self) -> SessionKey:
        """The direction independent key of this session."""
        return SessionKey.of(
            self.transport,
            self.client_ip,
            self.client_port,
            self.server_ip,
            self.server_port,
        )


@dataclass
class CaptureSummary:
    """The session table of one capture together with read statistics."""

    format: str
    link_types: list[int]
    frames: int = 0
    skipped_frames: int = 0
    sessions: list[Session] = field(default_factory=list)


def _ip_sort_key(ip: str) -> tuple[int, int]:
    addr = ipaddress.ip_address(ip)
    return (addr.version, int(addr))


def read_frames(path: Path) -> tuple[str, Iterator[Frame]]:
    """Open a capture and return its format name and an iterator over its frames.

    The format is ``pcap`` or ``pcapng``. Iteration raises :class:`CaptureError`
    when a record header is truncated or inconsistent.
    """
    try:
        data = path.read_bytes()
    except OSError as exc:
        raise CaptureError(f"read capture {path}: {exc.strerror}") from exc
    if len(data) < 4:
        raise CaptureError(f"read capture {path}: file is shorter than a header")
    if data[:4] in _PCAP_MAGIC:
        return "pcap", _pcap_frames(path, data)
    if struct.unpack("<I", data[:4])[0] == _PCAPNG_SHB:
        return "pcapng", _pcapng_frames(path, data)
    raise CaptureError(f"read capture {path}: unknown magic {data[:4].hex()}")


def _pcap_frames(path: Path, data: bytes) -> Iterator[Frame]:
    endian, resolution = _PCAP_MAGIC[data[:4]]
    if len(data) < 24:
        raise CaptureError(f"read capture {path}: truncated pcap file header")
    link_type = struct.unpack(endian + "I", data[20:24])[0] & 0x0FFFFFFF
    offset = 24
    while offset < len(data):
        if offset + 16 > len(data):
            raise CaptureError(f"read capture {path}: truncated record header at offset {offset}")
        ts_sec, ts_frac, incl_len, _orig_len = struct.unpack(
            endian + "IIII", data[offset : offset + 16]
        )
        start = offset + 16
        end = start + incl_len
        if incl_len > _MAX_BLOCK or end > len(data):
            raise CaptureError(
                f"read capture {path}: record at offset {offset} claims {incl_len} bytes"
            )
        yield Frame(ts_sec + ts_frac * resolution, link_type, data[start:end])
        offset = end


def _pcapng_frames(path: Path, data: bytes) -> Iterator[Frame]:
    endian = "<"
    interfaces: list[tuple[int, float]] = []
    offset = 0
    while offset < len(data):
        if offset + 12 > len(data):
            raise CaptureError(f"read capture {path}: truncated block header at offset {offset}")
        block_type = struct.unpack("<I", data[offset : offset + 4])[0]
        if block_type == _PCAPNG_SHB:
            bom = data[offset + 8 : offset + 12]
            if struct.unpack("<I", bom)[0] == _PCAPNG_BOM:
                endian = "<"
            elif struct.unpack(">I", bom)[0] == _PCAPNG_BOM:
                endian = ">"
            else:
                raise CaptureError(f"read capture {path}: bad byte order mark at offset {offset}")
            interfaces = []
        block_len = struct.unpack(endian + "I", data[offset + 4 : offset + 8])[0]
        if block_len < 12 or block_len % 4 or offset + block_len > len(data):
            raise CaptureError(
                f"read capture {path}: block at offset {offset} has length {block_len}"
            )
        body = data[offset + 8 : offset + block_len - 4]
        if block_type == _PCAPNG_IDB:
            interfaces.append(_pcapng_interface(path, offset, endian, body))
        elif block_type == _PCAPNG_EPB:
            if len(body) < 20:
                raise CaptureError(
                    f"read capture {path}: truncated packet block at offset {offset}"
                )
            if_id, ts_high, ts_low, cap_len, _orig = struct.unpack(endian + "IIIII", body[:20])
            if if_id >= len(interfaces) or 20 + cap_len > len(body):
                raise CaptureError(
                    f"read capture {path}: packet block at offset {offset} is inconsistent"
                )
            link_type, resolution = interfaces[if_id]
            ticks = (ts_high << 32) | ts_low
            yield Frame(ticks * resolution, link_type, body[20 : 20 + cap_len])
        elif block_type == _PCAPNG_SPB:
            if not interfaces or len(body) < 4:
                raise CaptureError(
                    f"read capture {path}: simple packet block at offset {offset} is inconsistent"
                )
            orig_len = struct.unpack(endian + "I", body[:4])[0]
            link_type, _resolution = interfaces[0]
            yield Frame(0.0, link_type, body[4 : 4 + min(orig_len, len(body) - 4)])
        offset += block_len


def _pcapng_interface(path: Path, offset: int, endian: str, body: bytes) -> tuple[int, float]:
    if len(body) < 8:
        raise CaptureError(f"read capture {path}: truncated interface block at offset {offset}")
    link_type = struct.unpack(endian + "H", body[:2])[0]
    resolution = 1e-6
    pos = 8
    while pos + 4 <= len(body):
        code, length = struct.unpack(endian + "HH", body[pos : pos + 4])
        value = body[pos + 4 : pos + 4 + length]
        if code == 0:
            break
        if code == 9 and len(value) == 1:
            raw = value[0]
            resolution = 2.0 ** -(raw & 0x7F) if raw & 0x80 else 10.0 ** -(raw & 0x7F)
        pos += 4 + length + (-length % 4)
    return link_type, resolution


def decode(frame: Frame) -> Packet | None:
    """Decode a frame into a TCP or UDP packet, or return None for anything else.

    Truncated headers, other link types, other transports, and IP fragments after
    the first all return None, so the caller counts them as skipped.
    """
    segment = decode_segment(frame)
    return segment.packet if segment is not None else None


@dataclass(frozen=True)
class Segment:
    """A decoded packet together with its TCP sequence number and its payload bytes.

    The sequence number is 0 for UDP. The payload is cut at the end of the IP
    packet, so Ethernet padding never counts as payload.
    """

    packet: Packet
    seq: int
    payload: bytes


def decode_segment(frame: Frame) -> Segment | None:
    """Decode a frame like :func:`decode` and keep the sequence number and payload."""
    ip_data = _network_layer(frame)
    if ip_data is None:
        return None
    version, payload = ip_data
    parsed = _ipv4(payload) if version == 4 else _ipv6(payload)
    if parsed is None:
        return None
    proto, src, dst, segment = parsed
    if proto == _IP_TCP:
        if len(segment) < 20:
            return None
        sport, dport = struct.unpack("!HH", segment[:4])
        header_len = (segment[12] >> 4) * 4
        if header_len < 20 or header_len > len(segment):
            return None
        seq = struct.unpack("!I", segment[4:8])[0]
        body = segment[header_len:]
        packet = Packet(frame.timestamp, "tcp", src, sport, dst, dport, len(body), segment[13])
        return Segment(packet, seq, body)
    if proto == _IP_UDP:
        if len(segment) < 8:
            return None
        sport, dport = struct.unpack("!HH", segment[:4])
        body = segment[8:]
        packet = Packet(frame.timestamp, "udp", src, sport, dst, dport, len(body))
        return Segment(packet, 0, body)
    return None


def _network_layer(frame: Frame) -> tuple[int, bytes] | None:
    data = frame.data
    if frame.link_type == LINKTYPE_ETHERNET:
        if len(data) < 14:
            return None
        ethertype = struct.unpack("!H", data[12:14])[0]
        pos = 14
        while ethertype in _ETH_VLAN:
            if len(data) < pos + 4:
                return None
            ethertype = struct.unpack("!H", data[pos + 2 : pos + 4])[0]
            pos += 4
        return _by_ethertype(ethertype, data[pos:])
    if frame.link_type == LINKTYPE_LINUX_SLL:
        if len(data) < 16:
            return None
        return _by_ethertype(struct.unpack("!H", data[14:16])[0], data[16:])
    if frame.link_type == LINKTYPE_NULL:
        if len(data) < 4:
            return None
        family = struct.unpack("<I", data[:4])[0]
        if family not in (2, 24, 28, 30):
            family = struct.unpack(">I", data[:4])[0]
        if family == 2:
            return 4, data[4:]
        if family in (24, 28, 30):
            return 6, data[4:]
        return None
    if frame.link_type in (LINKTYPE_RAW, LINKTYPE_IPV4, LINKTYPE_IPV6):
        if not data:
            return None
        version = data[0] >> 4
        return (version, data) if version in (4, 6) else None
    return None


def _by_ethertype(ethertype: int, data: bytes) -> tuple[int, bytes] | None:
    if ethertype == _ETH_IPV4:
        return 4, data
    if ethertype == _ETH_IPV6:
        return 6, data
    return None


def _ipv4(data: bytes) -> tuple[int, str, str, bytes] | None:
    if len(data) < 20 or data[0] >> 4 != 4:
        return None
    header_len = (data[0] & 0x0F) * 4
    total_len = struct.unpack("!H", data[2:4])[0]
    if header_len < 20 or total_len < header_len or len(data) < header_len:
        return None
    fragment_offset = struct.unpack("!H", data[6:8])[0] & 0x1FFF
    if fragment_offset:
        return None
    src = str(ipaddress.IPv4Address(data[12:16]))
    dst = str(ipaddress.IPv4Address(data[16:20]))
    return data[9], src, dst, data[header_len : min(total_len, len(data))]


def _ipv6(data: bytes) -> tuple[int, str, str, bytes] | None:
    if len(data) < 40 or data[0] >> 4 != 6:
        return None
    payload_len = struct.unpack("!H", data[4:6])[0]
    next_header = data[6]
    src = str(ipaddress.IPv6Address(data[8:24]))
    dst = str(ipaddress.IPv6Address(data[24:40]))
    payload = data[40 : 40 + payload_len]
    while next_header in _IPV6_EXTENSION_HEADERS:
        if len(payload) < 8:
            return None
        ext_len = (payload[1] + 1) * 8
        if ext_len > len(payload):
            return None
        next_header = payload[0]
        payload = payload[ext_len:]
    if next_header == 44:  # fragment header, which this reader does not reassemble
        return None
    return next_header, src, dst, payload


def summarize(path: Path) -> CaptureSummary:
    """Read a capture and build its session table in order of first packet."""
    fmt, frames = read_frames(path)
    summary = CaptureSummary(format=fmt, link_types=[])
    by_key: dict[SessionKey, Session] = {}
    for frame in frames:
        summary.frames += 1
        if frame.link_type not in summary.link_types:
            summary.link_types.append(frame.link_type)
        packet = decode(frame)
        if packet is None:
            summary.skipped_frames += 1
            continue
        key = SessionKey.of(
            packet.transport,
            packet.src_ip,
            packet.src_port,
            packet.dst_ip,
            packet.dst_port,
        )
        session = by_key.get(key)
        if session is None:
            session = Session(
                id=len(by_key) + 1,
                transport=packet.transport,
                client_ip=packet.src_ip,
                client_port=packet.src_port,
                server_ip=packet.dst_ip,
                server_port=packet.dst_port,
                first_seen=packet.timestamp,
                last_seen=packet.timestamp,
            )
            by_key[key] = session
            summary.sessions.append(session)
        _orient(session, packet)
        session.packets += 1
        session.payload_bytes += packet.payload_len
        session.first_seen = min(session.first_seen, packet.timestamp)
        session.last_seen = max(session.last_seen, packet.timestamp)
    return summary


def _orient(session: Session, packet: Packet) -> None:
    """Make the sender of the first SYN without ACK the client of a TCP session."""
    if session.client_known or packet.transport != "tcp":
        return
    flags = packet.tcp_flags
    if flags & _TCP_SYN and not flags & _TCP_ACK:
        session.client_ip, session.client_port = packet.src_ip, packet.src_port
        session.server_ip, session.server_port = packet.dst_ip, packet.dst_port
        session.client_known = True
    elif flags & _TCP_SYN and flags & _TCP_ACK:
        session.client_ip, session.client_port = packet.dst_ip, packet.dst_port
        session.server_ip, session.server_port = packet.src_ip, packet.src_port
        session.client_known = True
