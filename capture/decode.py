# SPDX-License-Identifier: GPL-3.0-only
"""Decode link, network, and transport headers of captured frames.

The decoder never raises on frame content. A frame that it cannot use comes back as a
:class:`Skip` with a reason, and the caller counts the reasons.
"""

from __future__ import annotations

import socket
import struct
from dataclasses import dataclass

from .pcapio import Frame

LINKTYPE_NULL = 0
LINKTYPE_ETHERNET = 1
LINKTYPE_RAW = 101
LINKTYPE_LINUX_SLL = 113
LINKTYPE_IPV4 = 228
LINKTYPE_IPV6 = 229
LINKTYPE_LINUX_SLL2 = 276

TCP_FIN = 0x01
TCP_SYN = 0x02
TCP_RST = 0x04
TCP_ACK = 0x10

_ETH_IPV4 = 0x0800
_ETH_IPV6 = 0x86DD
_ETH_VLAN = (0x8100, 0x88A8, 0x9100)
_IP_TCP = 6
_IP_UDP = 17
_V6_SKIPPABLE = (0, 43, 60)
_V6_AH = 51
_V6_FRAGMENT = 44


@dataclass(frozen=True)
class Skip:
    """A frame that the decoder did not turn into a packet, with the reason."""

    reason: str


@dataclass(slots=True)
class Packet:
    """The transport view of one TCP segment or UDP datagram.

    ``payload_len`` is the length on the wire, taken from the IP header. ``payload`` holds
    what the capture kept, which is shorter when the snap length cut the frame.
    """

    ts_ns: int
    interface: int
    proto: str
    version: int
    src: bytes
    dst: bytes
    sport: int
    dport: int
    ip_len: int
    payload_len: int
    payload: bytes
    truncated: bool
    seq: int = 0
    ack: int = 0
    flags: int = 0


def address(raw: bytes) -> str:
    """Format a packed IPv4 or IPv6 address."""
    family = socket.AF_INET if len(raw) == 4 else socket.AF_INET6
    return socket.inet_ntop(family, raw)


def decode(frame: Frame) -> Packet | Skip:
    """Decode one frame to a packet, or say why it was skipped."""
    ip = _strip_link(frame.link_type, frame.data)
    if isinstance(ip, Skip):
        return ip
    if not ip:
        return Skip("truncated_header")
    version = ip[0] >> 4
    if version == 4:
        return _ipv4(frame, ip)
    if version == 6:
        return _ipv6(frame, ip)
    return Skip("not_ip")


def _strip_link(link_type: int, data: bytes) -> bytes | Skip:
    if link_type == LINKTYPE_ETHERNET:
        if len(data) < 14:
            return Skip("truncated_header")
        pos = 12
        ethertype = struct.unpack_from(">H", data, pos)[0]
        while ethertype in _ETH_VLAN:
            pos += 4
            if len(data) < pos + 2:
                return Skip("truncated_header")
            ethertype = struct.unpack_from(">H", data, pos)[0]
        if ethertype not in (_ETH_IPV4, _ETH_IPV6):
            return Skip("not_ip")
        return data[pos + 2 :]
    if link_type == LINKTYPE_NULL:
        return data[4:]
    if link_type in (LINKTYPE_RAW, LINKTYPE_IPV4, LINKTYPE_IPV6):
        return data
    if link_type == LINKTYPE_LINUX_SLL:
        if len(data) < 16:
            return Skip("truncated_header")
        proto = struct.unpack_from(">H", data, 14)[0]
        return data[16:] if proto in (_ETH_IPV4, _ETH_IPV6) else Skip("not_ip")
    if link_type == LINKTYPE_LINUX_SLL2:
        if len(data) < 20:
            return Skip("truncated_header")
        proto = struct.unpack_from(">H", data, 0)[0]
        return data[20:] if proto in (_ETH_IPV4, _ETH_IPV6) else Skip("not_ip")
    return Skip("unsupported_link_type")


def _ipv4(frame: Frame, ip: bytes) -> Packet | Skip:
    if len(ip) < 20:
        return Skip("truncated_header")
    ihl = (ip[0] & 0x0F) * 4
    total = struct.unpack_from(">H", ip, 2)[0]
    frag = struct.unpack_from(">H", ip, 6)[0]
    if ihl < 20 or len(ip) < ihl:
        return Skip("bad_header")
    if frag & 0x3FFF:
        return Skip("fragment")
    if total == 0:  # segmentation offload leaves a zero length
        total = len(ip)
    if total < ihl:
        return Skip("bad_header")
    return _transport(frame, 4, ip[12:16], ip[16:20], ip[9], ip[ihl:], total - ihl, total)


def _ipv6(frame: Frame, ip: bytes) -> Packet | Skip:
    if len(ip) < 40:
        return Skip("truncated_header")
    plen = struct.unpack_from(">H", ip, 4)[0]
    nxt = ip[6]
    pos = 40
    wire = plen if plen else len(ip) - 40
    for _ in range(8):
        if nxt in _V6_SKIPPABLE or nxt == _V6_AH:
            if len(ip) < pos + 2:
                return Skip("truncated_header")
            size = (ip[pos + 1] + 2) * 4 if nxt == _V6_AH else (ip[pos + 1] + 1) * 8
            nxt = ip[pos]
            pos += size
            wire -= size
        elif nxt == _V6_FRAGMENT:
            if len(ip) < pos + 8:
                return Skip("truncated_header")
            if struct.unpack_from(">H", ip, pos + 2)[0] & 0xFFF9:
                return Skip("fragment")
            nxt = ip[pos]
            pos += 8
            wire -= 8
        else:
            break
    if wire < 0 or pos > len(ip):
        return Skip("bad_header")
    return _transport(frame, 6, ip[8:24], ip[24:40], nxt, ip[pos:], wire, 40 + plen)


def _transport(
    frame: Frame,
    version: int,
    src: bytes,
    dst: bytes,
    proto: int,
    l4: bytes,
    l4_wire: int,
    ip_len: int,
) -> Packet | Skip:
    if proto == _IP_TCP:
        if len(l4) < 20:
            return Skip("truncated_header")
        sport, dport, seq, ack, off_byte, flags = struct.unpack_from(">HHIIBB", l4, 0)
        hdr = (off_byte >> 4) * 4
        if hdr < 20 or len(l4) < hdr or l4_wire < hdr:
            return Skip("bad_header")
        wire = l4_wire - hdr
        payload = l4[hdr : hdr + wire]
        return Packet(
            frame.ts_ns,
            frame.interface,
            "tcp",
            version,
            src,
            dst,
            sport,
            dport,
            ip_len,
            wire,
            payload,
            len(payload) < wire,
            seq,
            ack,
            flags,
        )
    if proto == _IP_UDP:
        if len(l4) < 8:
            return Skip("truncated_header")
        sport, dport, length = struct.unpack_from(">HHH", l4, 0)
        wire = length - 8 if 8 <= length <= l4_wire else l4_wire - 8
        if wire < 0:
            return Skip("bad_header")
        payload = l4[8 : 8 + wire]
        return Packet(
            frame.ts_ns,
            frame.interface,
            "udp",
            version,
            src,
            dst,
            sport,
            dport,
            ip_len,
            wire,
            payload,
            len(payload) < wire,
        )
    return Skip("unsupported_l4")
