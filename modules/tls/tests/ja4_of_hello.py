# SPDX-License-Identifier: GPL-3.0-only
"""Print the JA4 and the wire fields of the first ClientHello in a file of TCP stream bytes.

Usage: python3 -I ja4_of_hello.py <repository root> <file>

The file holds the bytes that a client wrote, starting with the TLS handshake
record. The script uses ``realism/ja4.py``, which reads the ClientHello bytes
itself, so the result does not depend on a tool that prints a fingerprint. It
prints one ``key=value`` line per field, with hex values separated by commas.
"""

import sys
from collections.abc import Sequence

if len(sys.argv) != 3:
    sys.exit("usage: ja4_of_hello.py <repository root> <file>")

sys.path.insert(0, sys.argv[1])

from realism import ja4

with open(sys.argv[2], "rb") as stream:
    hello = ja4.parse_client_hello(stream.read())


def hexes(values: Sequence[int]) -> str:
    return ",".join(f"{value:04x}" for value in values)


print(f"ja4={ja4.compute(hello)}")
print(f"ciphers={hexes(hello.cipher_suites)}")
print(f"extensions={hexes(hello.extensions)}")
print(f"sig_algs={hexes(hello.signature_algorithms)}")
print(f"alpn={','.join(p.decode('ascii', 'replace') for p in hello.alpn)}")
print(f"server_name={hello.server_name or ''}")
