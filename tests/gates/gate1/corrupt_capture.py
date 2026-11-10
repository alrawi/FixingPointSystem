# SPDX-License-Identifier: GPL-3.0-only
"""Corrupt one HTTP message of a capture, for the negative proof of the suite.

    python3 -I corrupt_capture.py IN.pcapng OUT.pcapng

The script copies the capture and overwrites the first chunk size line of one
chunked HTTP response, from the middle of the capture on, with a chunk size of
seventeen hex digits, which no 64 bit chunk size holds. The overwrite keeps the
length of every frame, so the TCP stream stays consistent and only the HTTP
message is corrupt. tshark reports the frame as a malformed packet, and Zeek
raises a weird for the chunk size. The script prints the number of the corrupted
frame and exits with 0, with 1 when the capture holds no chunked response after
its middle, and with 2 when an input is unreadable.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from parser_findings import ROOT, pcapng_bytes

sys.path.insert(0, str(ROOT))
from realism import capture as cap

MARKER = b"Transfer-Encoding: chunked\r\n"
BODY_START = b"\r\n\r\n"
SIZE = b"f" * 17 + b"\r\n"


def corrupt(frames: list[cap.Frame]) -> tuple[list[cap.Frame], int] | None:
    """Return the frames with one corrupted chunk size and its frame number."""
    for number, frame in enumerate(frames, start=1):
        if number <= len(frames) // 2:
            continue
        segment = cap.decode_segment(frame)
        if (
            segment is None
            or segment.packet.transport != "tcp"
            or MARKER not in segment.payload
        ):
            continue
        payload = segment.payload
        end = payload.find(BODY_START)
        if end < 0:
            continue
        start = end + len(BODY_START)
        if len(payload) - start < len(SIZE):
            continue
        # The payload ends the IP packet, and Ethernet padding follows it.
        offset = frame.data.find(payload)
        if offset < 0:
            continue
        data = (
            frame.data[: offset + start]
            + SIZE
            + frame.data[offset + start + len(SIZE) :]
        )
        frames = list(frames)
        frames[number - 1] = cap.Frame(frame.timestamp, frame.link_type, data)
        return frames, number
    return None


def main(argv: list[str] | None = None) -> int:
    """Run the command line interface."""
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("source", type=Path)
    p.add_argument("target", type=Path)
    args = p.parse_args(argv)
    try:
        _, frames = cap.read_frames(args.source)
        result = corrupt(list(frames))
    except cap.CaptureError as e:
        print(f"corrupt_capture.py: {e}", file=sys.stderr)
        return 2
    if result is None:
        print(
            "corrupt_capture.py: no chunked response after the middle of the capture",
            file=sys.stderr,
        )
        return 1
    corrupted, number = result
    args.target.write_bytes(
        pcapng_bytes(corrupted[0].link_type, [(f.timestamp, f.data) for f in corrupted])
    )
    print(number)
    return 0


if __name__ == "__main__":
    sys.exit(main())
