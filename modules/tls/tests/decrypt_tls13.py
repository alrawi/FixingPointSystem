# SPDX-License-Identifier: GPL-3.0-only
"""Decrypt the application data of a TLS 1.3 capture with an NSS key log.

This is an independent check of the key log export, which needs only Python and
the `cryptography` package and no tshark. It reads the classic pcap that the
capture test writes (one TCP connection, Ethernet and IPv4), takes the
ClientHello random from the capture, looks up the traffic secrets of that random
in the key log, derives record keys as RFC 8446 section 7.3 describes, and prints
every decrypted application data record as `c2s: <text>` or `s2c: <text>`.

Usage: decrypt_tls13.py CAPTURE.pcap KEYS.log
Exit status 0 means at least one record decrypted, 1 means none did.
"""

import hmac
import struct
import sys

from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM, ChaCha20Poly1305

# cipher suite id -> (hash name, key length, AEAD class)
SUITES = {
    0x1301: ("sha256", 16, AESGCM),
    0x1302: ("sha384", 32, AESGCM),
    0x1303: ("sha256", 32, ChaCha20Poly1305),
}


def read_streams(path):
    """Return the TCP payload of the client and the server, in capture order."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] != b"\xd4\xc3\xb2\xa1":
        raise SystemExit("not a little endian classic pcap")
    pos, streams = 24, {True: b"", False: b""}
    while pos < len(data):
        incl = struct.unpack_from("<I", data, pos + 8)[0]
        frame = data[pos + 16 : pos + 16 + incl]
        pos += 16 + incl
        ihl = (frame[14] & 0x0F) * 4
        tcp = frame[14 + ihl :]
        sport = struct.unpack_from(">H", tcp, 0)[0]
        payload = tcp[(tcp[12] >> 4) * 4 :]
        streams[sport != 443] += payload  # True: from the client
    return streams[True], streams[False]


def records(stream):
    pos = 0
    while pos + 5 <= len(stream):
        length = struct.unpack_from(">H", stream, pos + 3)[0]
        yield stream[pos : pos + 5], stream[pos + 5 : pos + 5 + length]
        pos += 5 + length


def expand_label(hash_name, secret, label, length):
    full = b"tls13 " + label
    info = struct.pack(">H", length) + bytes([len(full)]) + full + b"\x00"
    # HKDF-Expand with a single block is enough for keys and IVs of 32 bytes or less.
    return hmac.new(secret, info + b"\x01", hash_name).digest()[:length]


def keys_for(hash_name, key_len, secret):
    return expand_label(hash_name, secret, b"key", key_len), expand_label(
        hash_name, secret, b"iv", 12
    )


def read_keylog(path, client_random):
    secrets = {}
    with open(path) as f:
        lines = f.readlines()
    for line in lines:
        parts = line.split()
        if len(parts) == 3 and parts[1] == client_random.hex():
            secrets[parts[0]] = bytes.fromhex(parts[2])
    return secrets


def decrypt_direction(stream, suite, handshake_secret, traffic_secret):
    hash_name, key_len, aead = SUITES[suite]
    stages = [
        keys_for(hash_name, key_len, s) for s in (handshake_secret, traffic_secret)
    ]
    stage, seq, out = 0, 0, []
    for header, body in records(stream):
        if header[0] != 23:
            continue
        while True:
            key, iv = stages[stage]
            nonce = bytes(a ^ b for a, b in zip(iv, seq.to_bytes(12, "big")))
            try:
                plain = aead(key).decrypt(nonce, body, header)
            except InvalidTag:
                if stage == 0:
                    stage, seq = 1, 0
                    continue
                raise SystemExit("a record failed to decrypt with the logged secrets")
            break
        seq += 1
        plain = plain.rstrip(b"\x00")
        if plain and plain[-1] == 23:
            out.append(plain[:-1])
    return out


def main():
    capture, keylog = sys.argv[1:3]
    client, server = read_streams(capture)
    # ClientHello: record header (5), handshake header (4), version (2), random (32).
    client_random = client[11:43]
    # ServerHello: the cipher suite follows the session id.
    sid_len = server[43]
    suite = struct.unpack_from(">H", server, 44 + sid_len)[0]
    secrets = read_keylog(keylog, client_random)
    needed = (
        "CLIENT_HANDSHAKE_TRAFFIC_SECRET",
        "CLIENT_TRAFFIC_SECRET_0",
        "SERVER_HANDSHAKE_TRAFFIC_SECRET",
        "SERVER_TRAFFIC_SECRET_0",
    )
    if not all(n in secrets for n in needed):
        print("the key log lacks the secrets of this ClientHello", file=sys.stderr)
        return 1
    found = 0
    for tag, stream, hs, app in (
        ("c2s", client, secrets[needed[0]], secrets[needed[1]]),
        ("s2c", server, secrets[needed[2]], secrets[needed[3]]),
    ):
        for text in decrypt_direction(stream, suite, hs, app):
            print(f"{tag}: {text.decode('utf-8', 'replace')!r}")
            found += 1
    return 0 if found else 1


if __name__ == "__main__":
    sys.exit(main())
