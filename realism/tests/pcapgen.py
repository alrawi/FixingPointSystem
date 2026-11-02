# SPDX-License-Identifier: GPL-3.0-only
"""Build small synthetic captures for the realism tests.

Every capture is generated from code at test time, so the repository never holds
captured traffic. The builders write classic pcap or pcapng with Ethernet framing
and correct IPv4, IPv6, TCP, and UDP checksums, which nDPI and tshark accept.
"""

from __future__ import annotations

import ipaddress
import struct
from dataclasses import dataclass, field
from pathlib import Path

SYN = 0x02
ACK = 0x10
PSH = 0x08
FIN = 0x01

_SRC_MAC = bytes.fromhex("020000000001")
_DST_MAC = bytes.fromhex("020000000002")


def _checksum(data: bytes) -> int:
    if len(data) % 2:
        data += b"\x00"
    total = sum(int(word) for word in struct.unpack(f"!{len(data) // 2}H", data))
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return ~total & 0xFFFF


def _ip_packet(src: str, dst: str, proto: int, segment_without_sum: bytes, sum_at: int) -> bytes:
    s = ipaddress.ip_address(src)
    d = ipaddress.ip_address(dst)
    if s.version == 4:
        pseudo = s.packed + d.packed + struct.pack("!BBH", 0, proto, len(segment_without_sum))
    else:
        pseudo = s.packed + d.packed + struct.pack("!IxxxB", len(segment_without_sum), proto)
    csum = _checksum(pseudo + segment_without_sum)
    if proto == 17 and csum == 0:
        csum = 0xFFFF
    segment = (
        segment_without_sum[:sum_at] + struct.pack("!H", csum) + segment_without_sum[sum_at + 2 :]
    )
    if s.version == 4:
        header = struct.pack(
            "!BBHHHBBH4s4s", 0x45, 0, 20 + len(segment), 1, 0x4000, 64, proto, 0, s.packed, d.packed
        )
        header = header[:10] + struct.pack("!H", _checksum(header)) + header[12:]
        return header + segment
    header = struct.pack("!IHBB16s16s", 6 << 28, len(segment), proto, 64, s.packed, d.packed)
    return header + segment


def ethernet(ip_packet: bytes) -> bytes:
    """Wrap an IP packet in an Ethernet header with fixed local MAC addresses."""
    ethertype = 0x0800 if ip_packet[0] >> 4 == 4 else 0x86DD
    return _DST_MAC + _SRC_MAC + struct.pack("!H", ethertype) + ip_packet


def tcp(
    src: str, sport: int, dst: str, dport: int, flags: int, seq: int, ack: int, payload: bytes = b""
) -> bytes:
    """Build one IPv4 or IPv6 TCP packet with correct checksums."""
    segment = (
        struct.pack("!HHIIBBHHH", sport, dport, seq, ack, 5 << 4, flags, 65535, 0, 0) + payload
    )
    return _ip_packet(src, dst, 6, segment, 16)


def udp(src: str, sport: int, dst: str, dport: int, payload: bytes) -> bytes:
    """Build one IPv4 or IPv6 UDP datagram with correct checksums."""
    segment = struct.pack("!HHHH", sport, dport, 8 + len(payload), 0) + payload
    return _ip_packet(src, dst, 17, segment, 6)


@dataclass
class Capture:
    """An ordered list of timestamped Ethernet frames."""

    frames: list[tuple[float, bytes]] = field(default_factory=list)
    clock: float = 1_700_000_000.0

    def add(self, ip_packet: bytes, gap: float = 0.001) -> None:
        """Append an IP packet as an Ethernet frame one gap after the last frame."""
        self.clock += gap
        self.frames.append((self.clock, ethernet(ip_packet)))

    def tcp_session(
        self, client: tuple[str, int], server: tuple[str, int], request: bytes, response: bytes
    ) -> None:
        """Append a full TCP session with handshake, one exchange, and close."""
        (c, cp), (s, sp) = client, server
        cs, ss = 1000, 5000
        self.add(tcp(c, cp, s, sp, SYN, cs, 0))
        self.add(tcp(s, sp, c, cp, SYN | ACK, ss, cs + 1))
        self.add(tcp(c, cp, s, sp, ACK, cs + 1, ss + 1))
        self.add(tcp(c, cp, s, sp, PSH | ACK, cs + 1, ss + 1, request))
        self.add(tcp(s, sp, c, cp, ACK, ss + 1, cs + 1 + len(request)))
        self.add(tcp(s, sp, c, cp, PSH | ACK, ss + 1, cs + 1 + len(request), response))
        self.add(tcp(c, cp, s, sp, ACK, cs + 1 + len(request), ss + 1 + len(response)))
        self.add(tcp(c, cp, s, sp, FIN | ACK, cs + 1 + len(request), ss + 1 + len(response)))
        self.add(tcp(s, sp, c, cp, FIN | ACK, ss + 1 + len(response), cs + 2 + len(request)))
        self.add(tcp(c, cp, s, sp, ACK, cs + 2 + len(request), ss + 2 + len(response)))

    def http_get(
        self, client: tuple[str, int], server: tuple[str, int], path: str = "/index.html"
    ) -> None:
        """Append an HTTP/1.1 GET with a small HTML response."""
        body = b"<html><body>fps</body></html>"
        request = (
            f"GET {path} HTTP/1.1\r\nHost: www.example.com\r\n"
            "User-Agent: fps-test/0.1\r\nAccept: */*\r\n\r\n"
        ).encode()
        response = (
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: "
            + str(len(body)).encode()
            + b"\r\nServer: fps\r\n\r\n"
            + body
        )
        self.tcp_session(client, server, request, response)

    def dns_lookup(
        self,
        client: tuple[str, int],
        server: tuple[str, int],
        txid: int,
        name: str = "www.example.com",
    ) -> None:
        """Append a DNS A query and its answer."""
        qname = b"".join(bytes([len(p)]) + p.encode() for p in name.split(".")) + b"\x00"
        question = qname + struct.pack("!HH", 1, 1)
        query = struct.pack("!HHHHHH", txid, 0x0100, 1, 0, 0, 0) + question
        answer = struct.pack("!HHIH4s", 1, 1, 300, 4, bytes([192, 0, 2, 80]))
        reply = struct.pack("!HHHHHH", txid, 0x8180, 1, 1, 0, 0) + question + b"\xc0\x0c" + answer
        (c, cp), (s, sp) = client, server
        self.add(udp(c, cp, s, sp, query))
        self.add(udp(s, sp, c, cp, reply))

    def write_pcap(self, path: Path) -> Path:
        """Write the frames as a classic little endian pcap with microseconds."""
        out = [struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1)]
        for ts, frame in self.frames:
            sec = int(ts)
            usec = round((ts - sec) * 1_000_000)
            out.append(struct.pack("<IIII", sec, usec, len(frame), len(frame)) + frame)
        path.write_bytes(b"".join(out))
        return path

    def write_pcapng(self, path: Path) -> Path:
        """Write the frames as pcapng with one Ethernet interface at microseconds."""
        out = [_block(0x0A0D0D0A, struct.pack("<IHHq", 0x1A2B3C4D, 1, 0, -1))]
        out.append(_block(1, struct.pack("<HHI", 1, 0, 65535)))
        for ts, frame in self.frames:
            ticks = round(ts * 1_000_000)
            body = struct.pack("<IIIII", 0, ticks >> 32, ticks & 0xFFFFFFFF, len(frame), len(frame))
            body += frame + b"\x00" * (-len(frame) % 4)
            out.append(_block(6, body))
        path.write_bytes(b"".join(out))
        return path


def _block(block_type: int, body: bytes) -> bytes:
    length = 12 + len(body)
    return struct.pack("<II", block_type, length) + body + struct.pack("<I", length)


def mixed_capture(http_sessions: int = 3, dns_lookups: int = 3) -> Capture:
    """Build a capture with HTTP sessions to port 80 and DNS lookups to port 53."""
    cap = Capture()
    for i in range(http_sessions):
        cap.http_get(("10.0.0.10", 40000 + i), ("10.0.1.80", 80), f"/page{i}.html")
    for i in range(dns_lookups):
        cap.dns_lookup(("10.0.0.10", 50000 + i), ("10.0.1.53", 53), 0x1000 + i)
    return cap
