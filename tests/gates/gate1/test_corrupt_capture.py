# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of corrupt_capture.py, which need no root and no tools."""

from __future__ import annotations

from pathlib import Path

import corrupt_capture as cc
import parser_findings as pf
from test_parser_findings import ACK, CLIENT, PSH, SERVER, session, tcp_frame

from realism import capture as cap

CHUNKED = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1000\r\n" + b"a" * 100


def chunked_session(t: float, port: int) -> list[tuple[float, bytes]]:
    frames = session(t, port)
    frames[3] = tcp_frame(t + 0.003, SERVER, 80, CLIENT, port, PSH | ACK, CHUNKED)
    return frames


def frames_of(path: Path) -> list[cap.Frame]:
    return list(cap.read_frames(path)[1])


def test_corrupts_one_chunk_size_after_the_middle(tmp_path: Path) -> None:
    raw = (
        chunked_session(100.0, 40000)
        + chunked_session(110.0, 40001)
        + chunked_session(120.0, 40002)
    )
    source, target = tmp_path / "in.pcapng", tmp_path / "out.pcapng"
    source.write_bytes(pf.pcapng_bytes(1, sorted(raw)))
    assert cc.main([str(source), str(target)]) == 0
    before, after = frames_of(source), frames_of(target)
    changed = [i for i, (a, b) in enumerate(zip(before, after, strict=True)) if a != b]
    # Frame 10 holds the response of the second session, the first after the middle.
    assert changed == [9]
    assert len(before[9].data) == len(after[9].data)
    assert b"\r\n\r\n" + b"f" * 17 + b"\r\n" in after[9].data
    assert [f.timestamp for f in before] == [f.timestamp for f in after]


def test_capture_without_a_chunked_response_is_refused(tmp_path: Path) -> None:
    source, target = tmp_path / "in.pcapng", tmp_path / "out.pcapng"
    source.write_bytes(pf.pcapng_bytes(1, sorted(session(100.0, 40000))))
    assert cc.main([str(source), str(target)]) == 1
    assert not target.exists()


def test_unreadable_capture_is_reported(tmp_path: Path) -> None:
    source = tmp_path / "in.pcapng"
    source.write_bytes(b"not a capture")
    assert cc.main([str(source), str(tmp_path / "out.pcapng")]) == 2
