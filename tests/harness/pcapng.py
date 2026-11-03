# SPDX-License-Identifier: GPL-3.0-only
"""Minimal pcapng tools for the namespace harness.

The harness records with dumpcap when it is installed and otherwise with
tcpdump, which writes only pcap. This module converts such a pcap file to
pcapng and summarizes a pcapng file, so that tests can check a capture without
Wireshark. It uses only the standard library.

Usage:
    python3 -I pcapng.py convert IN.pcap OUT.pcapng
    python3 -I pcapng.py stats FILE.pcapng
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path

SHB_TYPE = 0x0A0D0D0A
IDB_TYPE = 0x00000001
SPB_TYPE = 0x00000003
EPB_TYPE = 0x00000006
BYTE_ORDER_MAGIC = 0x1A2B3C4D

# pcap magic numbers as read in little endian, with the timestamp resolution
# and the byte order they imply.
PCAP_MAGICS = {
    0xA1B2C3D4: ("<", 6),
    0xA1B23C4D: ("<", 9),
    0xD4C3B2A1: (">", 6),
    0x4D3CB2A1: (">", 9),
}

OPT_ENDOFOPT = 0
OPT_IF_TSRESOL = 9


class FormatError(ValueError):
    """The input is not a valid pcap or pcapng file."""


def _pad4(data: bytes) -> bytes:
    return data + b"\0" * (-len(data) % 4)


def _block(block_type: int, body: bytes) -> bytes:
    """Frame one little endian pcapng block around its body."""
    body = _pad4(body)
    total = 12 + len(body)
    return struct.pack("<II", block_type, total) + body + struct.pack("<I", total)


def section_header() -> bytes:
    """Return a section header block with an unspecified section length."""
    return _block(SHB_TYPE, struct.pack("<IHHq", BYTE_ORDER_MAGIC, 1, 0, -1))


def interface_description(linktype: int, snaplen: int, tsresol: int) -> bytes:
    """Return an interface description block with the if_tsresol option."""
    options = struct.pack("<HHB", OPT_IF_TSRESOL, 1, tsresol)
    options = _pad4(options) + struct.pack("<HH", OPT_ENDOFOPT, 0)
    return _block(IDB_TYPE, struct.pack("<HHI", linktype, 0, snaplen) + options)


def enhanced_packet(timestamp: int, data: bytes, orig_len: int) -> bytes:
    """Return an enhanced packet block on interface 0.

    The timestamp counts units of the interface's if_tsresol since the epoch.
    """
    header = struct.pack(
        "<IIIII",
        0,
        (timestamp >> 32) & 0xFFFFFFFF,
        timestamp & 0xFFFFFFFF,
        len(data),
        orig_len,
    )
    return _block(EPB_TYPE, header + data)


def pcap_to_pcapng(raw: bytes) -> bytes:
    """Convert the bytes of a pcap file to the bytes of a pcapng file."""
    if len(raw) < 24:
        raise FormatError("pcap file is shorter than its global header")
    (magic,) = struct.unpack("<I", raw[:4])
    if magic not in PCAP_MAGICS:
        raise FormatError(f"unknown pcap magic 0x{magic:08x}")
    order, tsresol = PCAP_MAGICS[magic]
    _, _, _, _, _, snaplen, linktype = struct.unpack(order + "IHHiIII", raw[:24])
    out = [section_header(), interface_description(linktype & 0xFFFF, snaplen, tsresol)]
    scale = 10**tsresol
    offset = 24
    while offset < len(raw):
        if offset + 16 > len(raw):
            raise FormatError("pcap file ends inside a record header")
        sec, frac, incl, orig = struct.unpack(order + "IIII", raw[offset : offset + 16])
        offset += 16
        if offset + incl > len(raw):
            raise FormatError("pcap file ends inside a record")
        out.append(
            enhanced_packet(sec * scale + frac, raw[offset : offset + incl], orig)
        )
        offset += incl
    return b"".join(out)


ETHERNET = 1
ETHERTYPE_VLAN = b"\x81\x00"


@dataclass
class Stats:
    """Summary of a pcapng file."""

    sections: int = 0
    linktypes: list[int] = field(default_factory=list)
    packets: int = 0
    max_orig_len: int = 0
    captured_bytes: int = 0
    vlan_tagged: int = 0

    def add(self, linktype: int, data: bytes, orig_len: int) -> None:
        """Count one packet with its captured data and original length."""
        self.packets += 1
        self.captured_bytes += len(data)
        self.max_orig_len = max(self.max_orig_len, orig_len)
        if linktype == ETHERNET and data[12:14] == ETHERTYPE_VLAN:
            self.vlan_tagged += 1


def pcapng_stats(raw: bytes) -> Stats:
    """Walk every block of a pcapng file and count its packets.

    The summary counts enhanced and simple packet blocks, which are the packet
    blocks that dumpcap and pcap_to_pcapng write.
    """
    stats = Stats()
    order = "<"
    section_linktypes: list[int] = []
    offset = 0
    while offset < len(raw):
        if offset + 12 > len(raw):
            raise FormatError("pcapng file ends inside a block header")
        if raw[offset : offset + 4] == struct.pack("<I", SHB_TYPE):
            (bom,) = struct.unpack("<I", raw[offset + 8 : offset + 12])
            if bom == BYTE_ORDER_MAGIC:
                order = "<"
            elif bom == struct.unpack(">I", struct.pack("<I", BYTE_ORDER_MAGIC))[0]:
                order = ">"
            else:
                raise FormatError("section header has a bad byte order magic")
            stats.sections += 1
            section_linktypes = []
        elif stats.sections == 0:
            raise FormatError("pcapng file does not start with a section header")
        block_type, total = struct.unpack(order + "II", raw[offset : offset + 8])
        if total < 12 or total % 4 or offset + total > len(raw):
            raise FormatError(f"block at offset {offset} has a bad length {total}")
        (trailer,) = struct.unpack(
            order + "I", raw[offset + total - 4 : offset + total]
        )
        if trailer != total:
            raise FormatError(f"block at offset {offset} has mismatched lengths")
        body = raw[offset + 8 : offset + total - 4]
        if block_type == IDB_TYPE:
            (linktype,) = struct.unpack(order + "H", body[:2])
            section_linktypes.append(linktype)
            stats.linktypes.append(linktype)
        elif block_type == EPB_TYPE:
            iface, _, _, incl, orig = struct.unpack(order + "IIIII", body[:20])
            if iface >= len(section_linktypes):
                raise FormatError(
                    f"packet at offset {offset} names unknown interface {iface}"
                )
            stats.add(section_linktypes[iface], body[20 : 20 + incl], orig)
        elif block_type == SPB_TYPE:
            if not section_linktypes:
                raise FormatError(f"packet at offset {offset} precedes every interface")
            (orig,) = struct.unpack(order + "I", body[:4])
            stats.add(section_linktypes[0], body[4 : 4 + orig], orig)
        offset += total
    if stats.sections == 0:
        raise FormatError("pcapng file is empty")
    return stats


def main(argv: list[str] | None = None) -> int:
    """Run the convert or stats command and return the exit status."""
    parser = argparse.ArgumentParser(
        description="Minimal pcapng tools for the namespace harness."
    )
    sub = parser.add_subparsers(dest="command", required=True)
    convert = sub.add_parser("convert", help="convert a pcap file to pcapng")
    convert.add_argument("source", type=Path)
    convert.add_argument("target", type=Path)
    stats = sub.add_parser("stats", help="print a JSON summary of a pcapng file")
    stats.add_argument("capture", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "convert":
            args.target.write_bytes(pcap_to_pcapng(args.source.read_bytes()))
        else:
            print(
                json.dumps(
                    asdict(pcapng_stats(args.capture.read_bytes())), sort_keys=True
                )
            )
    except (OSError, FormatError) as err:
        print(f"pcapng.py: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
