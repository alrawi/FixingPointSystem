# SPDX-License-Identifier: GPL-3.0-only
"""Build synthetic captures as bytes: Ethernet frames, TCP and UDP, pcap and pcapng."""

from __future__ import annotations

import socket
import struct
from dataclasses import dataclass
from decimal import Decimal

SYN, ACK, FIN, RST, PSH = 0x02, 0x10, 0x01, 0x04, 0x08
NS = 1_000_000_000
MAC_A = bytes.fromhex("020000000001")
MAC_B = bytes.fromhex("020000000002")


def tcp_frame(
    src: str,
    dst: str,
    sport: int,
    dport: int,
    *,
    seq: int = 0,
    ack: int = 0,
    flags: int = ACK,
    payload: bytes = b"",
    link: str = "eth",
) -> bytes:
    """Return one TCP segment in an Ethernet frame, or a raw IP packet with link="raw"."""
    tcp = struct.pack(
        ">HHIIBBHHH", sport, dport, seq % 2**32, ack % 2**32, 5 << 4, flags, 65535, 0, 0
    )
    return _ip(src, dst, 6, tcp + payload, link)


def udp_frame(
    src: str, dst: str, sport: int, dport: int, payload: bytes = b"", link: str = "eth"
) -> bytes:
    """Return one UDP datagram in an Ethernet frame."""
    udp = struct.pack(">HHHH", sport, dport, 8 + len(payload), 0)
    return _ip(src, dst, 17, udp + payload, link)


def _ip(src: str, dst: str, proto: int, l4: bytes, link: str) -> bytes:
    if ":" in src:
        ip = struct.pack(">IHBB", 6 << 28, len(l4), proto, 64)
        ip += socket.inet_pton(socket.AF_INET6, src) + socket.inet_pton(socket.AF_INET6, dst)
        ethertype = 0x86DD
    else:
        ip = struct.pack(">BBHHHBBH", 0x45, 0, 20 + len(l4), 1, 0, 64, proto, 0)
        ip += socket.inet_aton(src) + socket.inet_aton(dst)
        ethertype = 0x0800
    packet = ip + l4
    if link == "raw":
        return packet
    return MAC_B + MAC_A + struct.pack(">H", ethertype) + packet


@dataclass
class Rec:
    """One record to write: a time in seconds, a frame, and an interface index."""

    t: float
    frame: bytes
    iface: int = 0


def write_pcap(
    records: list[Rec], *, nano: bool = False, big: bool = False, link: int = 1
) -> bytes:
    """Return a classic pcap file."""
    e = ">" if big else "<"
    magic = 0xA1B23C4D if nano else 0xA1B2C3D4
    out = [struct.pack(e + "IHHiIII", magic, 2, 4, 0, 0, 65535, link)]
    for r in records:
        per = NS if nano else 1_000_000
        ticks = int(Decimal(str(r.t)) * per)
        out.append(struct.pack(e + "IIII", ticks // per, ticks % per, len(r.frame), len(r.frame)))
        out.append(r.frame)
    return b"".join(out)


def _block(btype: int, body: bytes, e: str = "<") -> bytes:
    pad = -len(body) % 4
    total = 12 + len(body) + pad
    return struct.pack(e + "II", btype, total) + body + bytes(pad) + struct.pack(e + "I", total)


def _opt(code: int, value: bytes, e: str = "<") -> bytes:
    return struct.pack(e + "HH", code, len(value)) + value + bytes(-len(value) % 4)


def pcapng_section(
    interfaces: list[tuple[int, int]],
    records: list[Rec],
    *,
    names: list[str] | None = None,
    big: bool = False,
) -> bytes:
    """Return one pcapng section.

    ``interfaces`` lists (link type, if_tsresol byte) pairs. A record names its interface
    by index, and its time is converted with that interface's resolution.
    """
    e = ">" if big else "<"
    out = _block(0x0A0D0D0A, struct.pack(e + "IHHq", 0x1A2B3C4D, 1, 0, -1), e)
    for n, (link, resol) in enumerate(interfaces):
        body = struct.pack(e + "HHI", link, 0, 262144)
        if names:
            body += _opt(2, names[n].encode(), e)
        body += _opt(9, bytes([resol]), e) + _opt(0, b"", e)
        out += _block(1, body, e)
    for r in records:
        resol = interfaces[r.iface][1]
        ticks_per_sec = (1 << (resol & 0x7F)) if resol & 0x80 else 10 ** (resol & 0x7F)
        ticks = int(Decimal(str(r.t)) * ticks_per_sec)
        body = struct.pack(
            e + "IIIII", r.iface, ticks >> 32, ticks & 0xFFFFFFFF, len(r.frame), len(r.frame)
        )
        out += _block(6, body + r.frame, e)
    return out
