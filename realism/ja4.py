# SPDX-License-Identifier: GPL-3.0-only
"""JA4 fingerprints of TLS ClientHello messages, written from the FoxIO JA4 specification.

``compute`` turns a parsed ClientHello into the TCP form of JA4, which reads
``a_b_c``. Part a holds the protocol, the TLS version, the SNI flag, the cipher
and extension counts, and the first and last character of the first ALPN value.
Part b is the truncated SHA-256 of the sorted cipher suites, and part c is the
truncated SHA-256 of the sorted extensions without SNI and ALPN, followed by the
signature algorithms in their original order. ``parse_client_hello`` reads the
bytes of a ClientHello, so the check works from the capture and not from the
fingerprint that a tool printed.

The module follows https://github.com/FoxIO-LLC/ja4/blob/main/technical_details/JA4.md
and uses only the standard library.
"""

from __future__ import annotations

import hashlib
import struct
from collections.abc import Sequence
from dataclasses import dataclass, field

EXT_SERVER_NAME = 0x0000
EXT_ALPN = 0x0010
EXT_SIGNATURE_ALGORITHMS = 0x000D
EXT_SUPPORTED_VERSIONS = 0x002B

_VERSIONS = {
    0x0304: "13",
    0x0303: "12",
    0x0302: "11",
    0x0301: "10",
    0x0300: "s3",
    0x0002: "s2",
    0xFEFF: "d1",
    0xFEFD: "d2",
    0xFEFC: "d3",
}
_EMPTY_HASH = "000000000000"


class ClientHelloError(ValueError):
    """The bytes are not a ClientHello that this module can read."""


def is_grease(value: int) -> bool:
    """Return True for a GREASE value of RFC 8701, where both bytes are 0xNA with the same N."""
    return value & 0x0F0F == 0x0A0A and (value >> 8) == (value & 0xFF)


@dataclass(frozen=True)
class ClientHello:
    """The fields of a ClientHello that JA4 reads.

    ``extensions`` lists the extension types in wire order. ``alpn`` holds the
    protocol ids of the ALPN extension, and ``signature_algorithms`` the values of
    the signature algorithms extension (0x000d) in wire order. ``supported_versions``
    holds the values of extension 0x002b.
    """

    version: int
    cipher_suites: Sequence[int]
    extensions: Sequence[int]
    alpn: Sequence[bytes] = ()
    signature_algorithms: Sequence[int] = ()
    supported_versions: Sequence[int] = ()
    transport: str = "t"
    server_name: str | None = field(default=None, compare=False)


def _sha12(text: str) -> str:
    return hashlib.sha256(text.encode("ascii")).hexdigest()[:12]


def _hex(values: Sequence[int]) -> list[str]:
    return [f"{value:04x}" for value in values]


def _version(hello: ClientHello) -> str:
    offered = [v for v in hello.supported_versions if not is_grease(v)]
    chosen = max(offered) if offered else hello.version
    return _VERSIONS.get(chosen, "00")


def _alpn(hello: ClientHello) -> str:
    if not hello.alpn or not hello.alpn[0]:
        return "00"
    first = hello.alpn[0]
    ends = (first[0], first[-1])
    if all(chr(b).isascii() and chr(b).isalnum() for b in ends):
        return chr(first[0]) + chr(first[-1])
    digits = first.hex()
    return digits[0] + digits[-1]


def parts(hello: ClientHello) -> tuple[str, str, str]:
    """Return the three parts of the JA4 fingerprint of a ClientHello."""
    ciphers = [c for c in hello.cipher_suites if not is_grease(c)]
    extensions = [e for e in hello.extensions if not is_grease(e)]
    a = (
        f"{hello.transport}{_version(hello)}"
        f"{'d' if EXT_SERVER_NAME in extensions else 'i'}"
        f"{min(len(ciphers), 99):02d}{min(len(extensions), 99):02d}{_alpn(hello)}"
    )
    b = _sha12(",".join(_hex(sorted(ciphers)))) if ciphers else _EMPTY_HASH
    hashed = sorted(e for e in extensions if e not in (EXT_SERVER_NAME, EXT_ALPN))
    if not hashed:
        return a, b, _EMPTY_HASH
    text = ",".join(_hex(hashed))
    if EXT_SIGNATURE_ALGORITHMS in extensions:
        text += "_" + ",".join(_hex(hello.signature_algorithms))
    return a, b, _sha12(text)


def compute(hello: ClientHello) -> str:
    """Return the JA4 of a ClientHello, such as ``t13d1516h2_8daaf6152771_e5627efa2ab1``."""
    return "_".join(parts(hello))


def split(fingerprint: str) -> tuple[str, str, str] | None:
    """Split a JA4 string into its three parts, or return None when it has another shape."""
    pieces = fingerprint.split("_")
    if len(pieces) != 3 or len(pieces[0]) != 10 or len(pieces[1]) != 12 or len(pieces[2]) != 12:
        return None
    return pieces[0], pieces[1], pieces[2]


def explain_difference(observed: str, expected: str) -> str:
    """Name the parts of two JA4 strings that differ, in words that point at the cause."""
    got, want = split(observed), split(expected)
    if got is None or want is None:
        return f"{observed} is not {expected}"
    reasons = []
    if got[0] != want[0]:
        reasons.append(
            f"part a (version, SNI flag, cipher and extension counts, ALPN) is {got[0]}, "
            f"expected {want[0]}"
        )
    if got[1] != want[1]:
        reasons.append(f"cipher suite hash is {got[1]}, expected {want[1]}")
    if got[2] != want[2]:
        reasons.append(f"extension and signature algorithm hash is {got[2]}, expected {want[2]}")
    return "; ".join(reasons) if reasons else "the fingerprints are equal"


class _Reader:
    def __init__(self, data: bytes, what: str) -> None:
        self.data = data
        self.pos = 0
        self.what = what

    def take(self, count: int) -> bytes:
        if count < 0 or self.pos + count > len(self.data):
            raise ClientHelloError(f"{self.what} is cut short at byte {self.pos}")
        chunk = self.data[self.pos : self.pos + count]
        self.pos += count
        return chunk

    def uint(self, size: int) -> int:
        return int.from_bytes(self.take(size), "big")

    @property
    def left(self) -> int:
        return len(self.data) - self.pos


def parse_client_hello(data: bytes) -> ClientHello:
    """Read the first ClientHello of a TCP stream that starts with a TLS record.

    The bytes must start with a handshake record. The ClientHello may span several
    records, and the function joins their payloads. A ClientHello that the stream
    does not hold completely raises :class:`ClientHelloError`.
    """
    body = _handshake_bytes(data)
    msg = _Reader(body, "ClientHello")
    if msg.uint(1) != 1:
        raise ClientHelloError("the first handshake message is not a ClientHello")
    length = msg.uint(3)
    hello = _Reader(msg.take(length), "ClientHello")
    version = hello.uint(2)
    hello.take(32)  # random
    hello.take(hello.uint(1))  # session id
    suites_raw = hello.take(hello.uint(2))
    if len(suites_raw) % 2:
        raise ClientHelloError("the cipher suite list has an odd length")
    suites = list(struct.unpack(f">{len(suites_raw) // 2}H", suites_raw))
    hello.take(hello.uint(1))  # compression methods
    types: list[int] = []
    alpn: list[bytes] = []
    sig_algs: list[int] = []
    versions: list[int] = []
    server_name: str | None = None
    if hello.left:
        block = _Reader(hello.take(hello.uint(2)), "extensions")
        while block.left:
            ext_type = block.uint(2)
            ext = _Reader(block.take(block.uint(2)), f"extension {ext_type:#06x}")
            types.append(ext_type)
            if ext_type == EXT_ALPN:
                alpn = _alpn_list(ext)
            elif ext_type == EXT_SIGNATURE_ALGORITHMS:
                raw = ext.take(ext.uint(2))
                sig_algs = list(struct.unpack(f">{len(raw) // 2}H", raw[: len(raw) // 2 * 2]))
            elif ext_type == EXT_SUPPORTED_VERSIONS:
                raw = ext.take(ext.uint(1))
                versions = list(struct.unpack(f">{len(raw) // 2}H", raw[: len(raw) // 2 * 2]))
            elif ext_type == EXT_SERVER_NAME:
                server_name = _server_name(ext)
    return ClientHello(
        version=version,
        cipher_suites=suites,
        extensions=types,
        alpn=alpn,
        signature_algorithms=sig_algs,
        supported_versions=versions,
        server_name=server_name,
    )


def _alpn_list(ext: _Reader) -> list[bytes]:
    block = _Reader(ext.take(ext.uint(2)), "ALPN list")
    protocols = []
    while block.left:
        protocols.append(block.take(block.uint(1)))
    return protocols


def _server_name(ext: _Reader) -> str | None:
    if not ext.left:
        return None
    block = _Reader(ext.take(ext.uint(2)), "server name list")
    while block.left:
        kind = block.uint(1)
        name = block.take(block.uint(2))
        if kind == 0:
            return name.decode("ascii", "replace")
    return None


def _handshake_bytes(data: bytes) -> bytes:
    """Join the payloads of the leading handshake records until one ClientHello is complete."""
    pos = 0
    joined = b""
    while True:
        if pos + 5 > len(data):
            raise ClientHelloError("the stream ends before the ClientHello is complete")
        if data[pos] != 0x16:
            raise ClientHelloError("the stream does not start with a TLS handshake record")
        length = struct.unpack_from(">H", data, pos + 3)[0]
        if pos + 5 + length > len(data):
            raise ClientHelloError("the stream ends before the ClientHello is complete")
        joined += data[pos + 5 : pos + 5 + length]
        pos += 5 + length
        if len(joined) >= 4 and len(joined) >= 4 + int.from_bytes(joined[1:4], "big"):
            return joined
