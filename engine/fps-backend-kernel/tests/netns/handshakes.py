# SPDX-License-Identifier: GPL-3.0-only
"""Check the TCP handshakes of a pcapng capture against an OS profile.

The namespace capture test of the kernel backend records the client arm of
the QA-01 harness and calls this module once per profile. It finds the SYN
and the SYN-ACK of the session to one server port and compares the IPv4 TTL,
the do not fragment flag, the MSS, the window scale, and the SACK permitted
and timestamp options with the expected values. The SYN-ACK crossed the
routing namespace, so its TTL is one lower than the server sent. The module
uses only the standard library.

Usage:
    python3 -I handshakes.py FILE.pcapng --port PORT --ttl N --mss N
        [--wscale N] [--sack 0|1] [--ts 0|1] [--df 0|1] [--options mss,nop,...]

Only the TTL and the MSS are required, and the other checks run when given.
"""

from __future__ import annotations

import argparse
import struct
import sys
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path

SHB_TYPE = 0x0A0D0D0A
IDB_TYPE = 0x00000001
EPB_TYPE = 0x00000006
BYTE_ORDER_MAGIC = 0x1A2B3C4D
LINKTYPE_ETHERNET = 1

ETH_IPV4 = 0x0800
ETH_VLAN = (0x8100, 0x88A8)
IPPROTO_TCP = 6
TCP_SYN = 0x02
TCP_ACK = 0x10

OPTION_NAMES = {0: "eol", 1: "nop", 2: "mss", 3: "ws", 4: "sok", 8: "ts"}


class CaptureError(ValueError):
    """The capture is not a pcapng file that this module can read."""


@dataclass
class Handshake:
    """The fields of one SYN or SYN-ACK that a profile decides."""

    kind: str
    ttl: int
    df: bool
    window: int
    options: list[str] = field(default_factory=list)
    mss: int | None = None
    wscale: int | None = None

    def describe(self) -> str:
        """Return one line for the test log."""
        return (
            f"{self.kind} ttl={self.ttl} df={int(self.df)} window={self.window} "
            f"mss={self.mss} wscale={self.wscale} options={','.join(self.options)}"
        )


def ethernet_frames(raw: bytes) -> Iterator[bytes]:
    """Yield the frames of every Ethernet interface in a pcapng file."""
    pos = 0
    order = "<"
    linktypes: list[int] = []
    while pos + 12 <= len(raw):
        (block_type,) = struct.unpack_from(order + "I", raw, pos)
        if block_type == SHB_TYPE:
            if pos + 12 > len(raw):
                raise CaptureError("truncated section header")
            (magic,) = struct.unpack_from("<I", raw, pos + 8)
            if magic == BYTE_ORDER_MAGIC:
                order = "<"
            elif magic == struct.unpack("<I", struct.pack(">I", BYTE_ORDER_MAGIC))[0]:
                order = ">"
            else:
                raise CaptureError("bad byte order magic")
            linktypes = []
        (total,) = struct.unpack_from(order + "I", raw, pos + 4)
        if total < 12 or total % 4 or pos + total > len(raw):
            raise CaptureError(f"bad block length {total} at offset {pos}")
        body = raw[pos + 8 : pos + total - 4]
        if block_type == IDB_TYPE:
            if len(body) < 2:
                raise CaptureError("truncated interface description")
            (linktype,) = struct.unpack_from(order + "H", body, 0)
            linktypes.append(linktype)
        elif block_type == EPB_TYPE:
            if len(body) < 20:
                raise CaptureError("truncated enhanced packet")
            iface, _hi, _lo, caplen, _orig = struct.unpack_from(
                order + "IIIII", body, 0
            )
            if iface >= len(linktypes) or caplen > len(body) - 20:
                raise CaptureError(
                    "enhanced packet names a missing interface or overflows"
                )
            if linktypes[iface] == LINKTYPE_ETHERNET:
                yield body[20 : 20 + caplen]
        pos += total


def tcp_options(data: bytes) -> tuple[list[str], int | None, int | None]:
    """Return the option names, the MSS, and the window scale of a TCP header."""
    names: list[str] = []
    mss: int | None = None
    wscale: int | None = None
    i = 0
    while i < len(data):
        kind = data[i]
        names.append(OPTION_NAMES.get(kind, str(kind)))
        if kind == 0:
            break
        if kind == 1:
            i += 1
            continue
        if i + 1 >= len(data) or data[i + 1] < 2 or i + data[i + 1] > len(data):
            raise CaptureError("malformed TCP option")
        length = data[i + 1]
        if kind == 2 and length == 4:
            (mss,) = struct.unpack_from("!H", data, i + 2)
        elif kind == 3 and length == 3:
            wscale = data[i + 2]
        i += length
    return names, mss, wscale


def handshake(frame: bytes, port: int) -> Handshake | None:
    """Return the SYN to `port` or the SYN-ACK from `port`, else None."""
    pos = 12
    if len(frame) < pos + 2:
        return None
    (ethertype,) = struct.unpack_from("!H", frame, pos)
    while ethertype in ETH_VLAN and len(frame) >= pos + 6:
        pos += 4
        (ethertype,) = struct.unpack_from("!H", frame, pos)
    if ethertype != ETH_IPV4:
        return None
    ip = frame[pos + 2 :]
    if len(ip) < 20 or ip[0] >> 4 != 4 or ip[9] != IPPROTO_TCP:
        return None
    ihl = (ip[0] & 0x0F) * 4
    (frag,) = struct.unpack_from("!H", ip, 6)
    tcp = ip[ihl:]
    if len(tcp) < 20:
        return None
    sport, dport = struct.unpack_from("!HH", tcp, 0)
    flags = tcp[13]
    offset = (tcp[12] >> 4) * 4
    if offset < 20 or offset > len(tcp):
        raise CaptureError("bad TCP data offset")
    if flags & (TCP_SYN | TCP_ACK) == TCP_SYN and dport == port:
        kind = "syn"
    elif flags & (TCP_SYN | TCP_ACK) == TCP_SYN | TCP_ACK and sport == port:
        kind = "syn-ack"
    else:
        return None
    (window,) = struct.unpack_from("!H", tcp, 14)
    names, mss, wscale = tcp_options(tcp[20:offset])
    return Handshake(kind, ip[8], bool(frag & 0x4000), window, names, mss, wscale)


def check(hs: Handshake, args: argparse.Namespace, ttl: int) -> list[str]:
    """Return one message per field of `hs` that differs from the profile."""
    problems = []
    want: dict[str, object] = {"ttl": ttl, "mss": args.mss}
    if args.wscale is not None:
        want["wscale"] = args.wscale
    if args.sack is not None:
        want["sack"] = bool(args.sack)
    if args.ts is not None:
        want["ts"] = bool(args.ts)
    got: dict[str, object] = {
        "ttl": hs.ttl,
        "mss": hs.mss,
        "wscale": hs.wscale,
        "sack": "sok" in hs.options,
        "ts": "ts" in hs.options,
    }
    if args.df is not None:
        want["df"] = bool(args.df)
        got["df"] = hs.df
    for key, value in want.items():
        if got[key] != value:
            problems.append(f"{hs.kind} {key} is {got[key]}, want {value}")
    return problems


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("file", type=Path)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--ttl", type=int, required=True)
    parser.add_argument("--mss", type=int, required=True)
    parser.add_argument("--wscale", type=int)
    parser.add_argument("--sack", type=int, choices=(0, 1))
    parser.add_argument("--ts", type=int, choices=(0, 1))
    parser.add_argument("--df", type=int, choices=(0, 1))
    parser.add_argument(
        "--options", default="", help="reference option order, reported only"
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        found: dict[str, Handshake] = {}
        for frame in ethernet_frames(args.file.read_bytes()):
            hs = handshake(frame, args.port)
            if hs is not None:
                found.setdefault(hs.kind, hs)
    except (OSError, CaptureError) as e:
        print(f"handshakes: {args.file}: {e}", file=sys.stderr)
        return 2
    problems = []
    for kind, ttl in (("syn", args.ttl), ("syn-ack", args.ttl - 1)):
        hs = found.get(kind)
        if hs is None:
            problems.append(f"no {kind} for port {args.port}")
            continue
        print(hs.describe())
        problems.extend(check(hs, args, ttl))
        reference = [o for o in args.options.split(",") if o]
        if kind == "syn" and reference and hs.options != reference:
            print(
                f"note: Linux orders the options as {','.join(hs.options)}, "
                f"the reference system as {','.join(reference)}"
            )
    for p in problems:
        print(f"FAIL: {p}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
