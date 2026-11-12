# SPDX-License-Identifier: GPL-3.0-only
"""Split the first ClientHello of a classic pcap over two TCP segments.

    python3 -I split_client_hello.py IN.pcap OUT.pcap FIRST_BYTES

The segment that carries the first ClientHello of IN.pcap (an Ethernet and IPv4 frame
whose TCP payload starts with a TLS handshake record of type ClientHello) becomes two
frames in OUT.pcap, the first with FIRST_BYTES bytes of the payload and the second with
the rest. The sequence numbers and lengths follow. The checksums stay as they are,
which tshark does not verify. tshark_wrapper.rs uses it to show what the heuristic of
tshark does with a ClientHello that starts in a short segment (MOD-23).
"""

import struct
import sys

ETHERNET = 14


def records(body):
    offset = 0
    while offset < len(body):
        captured = struct.unpack("<I", body[offset + 8 : offset + 12])[0]
        yield body[offset : offset + 16 + captured]
        offset += 16 + captured


def client_hello(record):
    """Return (ip_header_length, tcp_header_length) when the record holds a ClientHello."""
    frame = record[16:]
    if len(frame) < ETHERNET + 20 or frame[12:14] != b"\x08\x00" or frame[ETHERNET + 9] != 6:
        return None
    ip_len = (frame[ETHERNET] & 15) * 4
    tcp_len = (frame[ETHERNET + ip_len + 12] >> 4) * 4
    payload = frame[ETHERNET + ip_len + tcp_len :]
    if len(payload) > 5 and payload[0] == 0x16 and payload[5] == 0x01:
        return ip_len, tcp_len
    return None


def piece(record, ip_len, tcp_len, start, chunk, ident):
    head = ETHERNET + ip_len + tcp_len
    frame = bytearray(record[16 : 16 + head]) + chunk
    struct.pack_into("!H", frame, ETHERNET + 2, ip_len + tcp_len + len(chunk))
    struct.pack_into("!H", frame, ETHERNET + 4, ident)
    seq_at = ETHERNET + ip_len + 4
    seq = struct.unpack_from("!I", frame, seq_at)[0]
    struct.pack_into("!I", frame, seq_at, (seq + start) & 0xFFFFFFFF)
    return record[:8] + struct.pack("<II", len(frame), len(frame)) + bytes(frame)


def main():
    source, target, first = sys.argv[1], sys.argv[2], int(sys.argv[3])
    data = open(source, "rb").read()
    header, body = data[:24], data[24:]
    out = []
    done = False
    for record in records(body):
        found = None if done else client_hello(record)
        if found is None:
            out.append(record)
            continue
        ip_len, tcp_len = found
        payload = record[16 + ETHERNET + ip_len + tcp_len :]
        out.append(piece(record, ip_len, tcp_len, 0, payload[:first], 0x7000))
        out.append(piece(record, ip_len, tcp_len, first, payload[first:], 0x7001))
        done = True
    if not done:
        sys.exit("no ClientHello in " + source)
    with open(target, "wb") as handle:
        handle.write(header + b"".join(out))


main()
