# SPDX-License-Identifier: GPL-3.0-only
"""Generate TLS captures with real handshakes for the encryption check tests.

The module runs a TLS client and server of the Python ``ssl`` module against each
other through memory buffers, so no network and no privileges are needed, and it
writes the exchanged bytes as TCP segments with :mod:`realism.tests.pcapgen`. The
client logs its secrets in the NSS format, which the tests embed in a pcapng
decryption secrets block or write as a sidecar file. A self signed certificate
comes from the ``openssl`` program, and tests skip when it is missing.
"""

from __future__ import annotations

import shutil
import ssl
import struct
import subprocess
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from realism import ja4
from realism.tests import pcapgen

MSS = 1400
SERVER_NAME = "tls.example.test"

HAS_OPENSSL = shutil.which("openssl") is not None


@dataclass(frozen=True)
class ClientConfig:
    """How the test client shapes its ClientHello."""

    alpn: Sequence[str] = ("h2", "http/1.1")
    ciphers: str | None = None
    max_version: ssl.TLSVersion = ssl.TLSVersion.MAXIMUM_SUPPORTED
    server_name: str | None = SERVER_NAME


def make_certificate(directory: Path) -> tuple[Path, Path]:
    """Write a self signed certificate and its key with openssl and return both paths."""
    cert, key = directory / "cert.pem", directory / "key.pem"
    argv = ["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt"]
    argv += ["ec_paramgen_curve:prime256v1", "-nodes", "-days", "2"]
    argv += ["-subj", f"/CN={SERVER_NAME}", "-addext", f"subjectAltName=DNS:{SERVER_NAME}"]
    argv += ["-keyout", str(key), "-out", str(cert)]
    subprocess.run(argv, check=True, capture_output=True)
    return cert, key


def handshake(
    cert: Path, key: Path, keylog: Path, config: ClientConfig
) -> list[tuple[bool, bytes]]:
    """Run a handshake, one request, and one response and return the flights.

    Each flight is a pair of a flag that tells whether the client sent it and its
    bytes. The client appends its secrets to ``keylog``.
    """
    server_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    server_ctx.load_cert_chain(str(cert), str(key))
    server_ctx.set_alpn_protocols(["h2", "http/1.1"])
    client_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    client_ctx.check_hostname = False
    client_ctx.verify_mode = ssl.CERT_NONE
    client_ctx.maximum_version = config.max_version
    client_ctx.keylog_filename = str(keylog)
    if config.alpn:
        client_ctx.set_alpn_protocols(list(config.alpn))
    if config.ciphers is not None:
        client_ctx.set_ciphers(config.ciphers)
    c_in, c_out, s_in, s_out = (ssl.MemoryBIO() for _ in range(4))
    client = client_ctx.wrap_bio(c_in, c_out, server_hostname=config.server_name)
    server = server_ctx.wrap_bio(s_in, s_out, server_side=True)
    flights: list[tuple[bool, bytes]] = []

    def pump() -> None:
        while True:
            moved = False
            for from_client, out, into in ((True, c_out, s_in), (False, s_out, c_in)):
                data = out.read()
                if data:
                    flights.append((from_client, data))
                    into.write(data)
                    moved = True
            if not moved:
                return

    done = {"c": False, "s": False}
    for _ in range(8):
        for name, side in (("c", client), ("s", server)):
            if not done[name]:
                try:
                    side.do_handshake()
                    done[name] = True
                except ssl.SSLWantReadError:
                    pass
            pump()
    client.write(b"GET /fps HTTP/1.1\r\nHost: tls.example.test\r\n\r\n")
    pump()
    server.read(1024)
    server.write(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi")
    pump()
    client.read(1024)
    return flights


def add_session(
    cap: pcapgen.Capture,
    client: tuple[str, int],
    server: tuple[str, int],
    flights: Sequence[tuple[bool, bytes]],
) -> None:
    """Append a TCP session that carries the flights, cut into segments of 1400 bytes."""
    (c, cp), (s, sp) = client, server
    cseq, sseq = 1000, 5000
    cap.add(pcapgen.tcp(c, cp, s, sp, pcapgen.SYN, cseq, 0))
    cap.add(pcapgen.tcp(s, sp, c, cp, pcapgen.SYN | pcapgen.ACK, sseq, cseq + 1))
    cap.add(pcapgen.tcp(c, cp, s, sp, pcapgen.ACK, cseq + 1, sseq + 1))
    cseq, sseq = cseq + 1, sseq + 1
    for from_client, data in flights:
        for start in range(0, len(data), MSS):
            segment = data[start : start + MSS]
            if from_client:
                cap.add(pcapgen.tcp(c, cp, s, sp, pcapgen.PSH | pcapgen.ACK, cseq, sseq, segment))
                cseq += len(segment)
            else:
                cap.add(pcapgen.tcp(s, sp, c, cp, pcapgen.PSH | pcapgen.ACK, sseq, cseq, segment))
                sseq += len(segment)
    cap.add(pcapgen.tcp(c, cp, s, sp, pcapgen.FIN | pcapgen.ACK, cseq, sseq))
    cap.add(pcapgen.tcp(s, sp, c, cp, pcapgen.FIN | pcapgen.ACK, sseq, cseq + 1))
    cap.add(pcapgen.tcp(c, cp, s, sp, pcapgen.ACK, cseq + 1, sseq + 1))


def tls_capture(
    directory: Path, configs: Sequence[ClientConfig], server_port: int = 443
) -> tuple[pcapgen.Capture, Path]:
    """Build a capture with one TLS session per config and return it with its key log path.

    Session ``i`` runs from ``10.0.0.10:40000+i`` to ``10.0.1.43:server_port``.
    """
    cert, key = make_certificate(directory)
    keylog = directory / "keys.log"
    keylog.write_text("")
    cap = pcapgen.Capture()
    for index, config in enumerate(configs):
        flights = handshake(cert, key, keylog, config)
        add_session(cap, ("10.0.0.10", 40000 + index), ("10.0.1.43", server_port), flights)
    return cap, keylog


def build_client_hello(hello: ja4.ClientHello, server_name: str | None = None) -> bytes:
    """Serialize a ClientHello into one TLS handshake record, for parser and stub tests."""
    ext: list[bytes] = []

    def add(kind: int, body: bytes) -> None:
        ext.append(struct.pack(">HH", kind, len(body)) + body)

    for kind in hello.extensions:
        if kind == ja4.EXT_SERVER_NAME:
            name = (server_name or "tls.example.test").encode()
            entry = b"\x00" + struct.pack(">H", len(name)) + name
            add(kind, struct.pack(">H", len(entry)) + entry)
        elif kind == ja4.EXT_ALPN:
            entries = b"".join(bytes([len(p)]) + p for p in hello.alpn)
            add(kind, struct.pack(">H", len(entries)) + entries)
        elif kind == ja4.EXT_SIGNATURE_ALGORITHMS:
            algs = b"".join(struct.pack(">H", a) for a in hello.signature_algorithms)
            add(kind, struct.pack(">H", len(algs)) + algs)
        elif kind == ja4.EXT_SUPPORTED_VERSIONS:
            versions = b"".join(struct.pack(">H", v) for v in hello.supported_versions)
            add(kind, bytes([len(versions)]) + versions)
        else:
            add(kind, b"\x00")
    suites = b"".join(struct.pack(">H", c) for c in hello.cipher_suites)
    extensions = b"".join(ext)
    body = struct.pack(">H", hello.version) + bytes(32) + b"\x00"
    body += struct.pack(">H", len(suites)) + suites + b"\x01\x00"
    body += struct.pack(">H", len(extensions)) + extensions
    handshake_msg = b"\x01" + len(body).to_bytes(3, "big") + body
    return b"\x16\x03\x01" + struct.pack(">H", len(handshake_msg)) + handshake_msg
