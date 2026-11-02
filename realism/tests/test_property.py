# SPDX-License-Identifier: GPL-3.0-only
"""Property tests of the capture reader over generated inputs."""

from __future__ import annotations

import struct
import tempfile
from pathlib import Path

from hypothesis import given, settings
from hypothesis import strategies as st

from realism.capture import LINKTYPE_ETHERNET, CaptureError, Frame, SessionKey, decode, summarize
from realism.tests import pcapgen

_PCAP_HEADER = struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1)
_PCAPNG_HEADER = struct.pack("<IIIHHqI", 0x0A0D0D0A, 28, 0x1A2B3C4D, 1, 0, -1, 28)

ipv4 = st.ip_addresses(v=4).map(str)
ports = st.integers(min_value=1, max_value=65535)


@settings(max_examples=200, deadline=None)
@given(tail=st.binary(max_size=512), pcapng=st.booleans())
def test_arbitrary_bytes_raise_only_capture_error(tail: bytes, pcapng: bool) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "fuzz.cap"
        path.write_bytes((_PCAPNG_HEADER if pcapng else _PCAP_HEADER) + tail)
        try:
            summary = summarize(path)
        except CaptureError:
            return
        assert summary.skipped_frames <= summary.frames


@settings(max_examples=300)
@given(data=st.binary(max_size=200))
def test_decode_never_raises_on_arbitrary_frames(data: bytes) -> None:
    decode(Frame(0.0, LINKTYPE_ETHERNET, b"\x02" * 12 + b"\x08\x00" + data))
    decode(Frame(0.0, LINKTYPE_ETHERNET, b"\x02" * 12 + b"\x86\xdd" + data))


@given(
    transport=st.sampled_from(["tcp", "udp"]),
    ip1=ipv4,
    port1=ports,
    ip2=ipv4,
    port2=ports,
)
def test_session_key_is_symmetric(
    transport: str, ip1: str, port1: int, ip2: str, port2: int
) -> None:
    assert SessionKey.of(transport, ip1, port1, ip2, port2) == SessionKey.of(
        transport, ip2, port2, ip1, port1
    )


@settings(max_examples=50, deadline=None)
@given(
    lookups=st.lists(
        st.tuples(ipv4, st.integers(1024, 65535)), min_size=1, max_size=20, unique=True
    )
)
def test_every_generated_lookup_becomes_one_session(lookups: list[tuple[str, int]]) -> None:
    cap = pcapgen.Capture()
    for i, (client, port) in enumerate(lookups):
        cap.dns_lookup((client, port), ("192.0.2.53", 53), i)
    with tempfile.TemporaryDirectory() as tmp:
        summary = summarize(cap.write_pcapng(Path(tmp) / "dns.pcapng"))
    assert len(summary.sessions) == len(lookups)
    assert all(s.server_port == 53 and s.packets == 2 for s in summary.sessions)
