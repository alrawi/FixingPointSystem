#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Checks that the RTP packets of a capture arrive at the configured interval.

The temporal check of the realism runner (realism/temporal.py) compares the gaps
between the starts of sessions and the payload sizes, so it cannot see the gaps
between the packets inside one RTP stream. This module check does. It reads every
RTP stream of the capture with tshark, takes the intended packet interval of each
stream from its RTP timestamps (the timestamp step divided by the clock rate of
the payload type, which is the packet time the sender configured), and requires
for each stream, in each direction:

* a mean inter arrival time within `--mean-tolerance` of the interval, which fails
  when the pacing drifts, as a plain sleep per packet does;
* at least `--within-share` of the gaps within `--jitter` of the interval, which
  tolerates the few late wake ups of a loaded host and still fails on bursts.

    python3 rtp_check.py CAPTURE [--mean-tolerance 0.03] [--jitter 0.25]
                         [--within-share 0.9] [--min-packets 10]
"""

from __future__ import annotations

import argparse
import statistics
import subprocess
import sys
from collections import defaultdict
from dataclasses import dataclass

# RTP clock rates of the payload types that the sip_rtp module streams (RFC 3551,
# RFC 7587). G.722 counts at 8000 hertz although it samples at 16000.
CLOCK_HZ = {0: 8000, 8: 8000, 9: 8000, 18: 8000, 111: 48000}

FIELDS = [
    "frame.time_epoch",
    "ip.src",
    "udp.srcport",
    "ip.dst",
    "udp.dstport",
    "rtp.ssrc",
    "rtp.p_type",
    "rtp.timestamp",
]


@dataclass(frozen=True)
class Packet:
    time: float
    stream: tuple
    payload_type: int
    timestamp: int


@dataclass
class StreamResult:
    stream: tuple
    packets: int
    interval_ms: float
    mean_ms: float
    within: float
    errors: list[str]


def load(pcap: str, tshark: str = "tshark") -> list[Packet]:
    """Returns the RTP packets of `pcap` in capture order."""
    cmd = [tshark, "-r", pcap, "-Y", "rtp", "-T", "fields", "-E", "separator=/t"]
    for f in FIELDS:
        cmd += ["-e", f]
    run = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if run.returncode != 0:
        sys.exit(f"rtp_check: tshark failed: {run.stderr.strip()}")
    return parse(run.stdout)


def parse(text: str) -> list[Packet]:
    packets = []
    for line in text.splitlines():
        cols = line.split("\t")
        if len(cols) != len(FIELDS) or not all(cols):
            continue
        packets.append(
            Packet(
                time=float(cols[0]),
                stream=(cols[1], cols[2], cols[3], cols[4], cols[5]),
                payload_type=int(cols[6].split(",")[0]),
                timestamp=int(cols[7].split(",")[0]),
            )
        )
    return packets


def interval_ms(packets: list[Packet]) -> float | None:
    """The packet time that the RTP timestamps of one stream announce."""
    clock = CLOCK_HZ.get(packets[0].payload_type)
    if clock is None:
        return None
    steps = [(b.timestamp - a.timestamp) % 2**32 for a, b in zip(packets, packets[1:])]
    return statistics.median(steps) * 1000 / clock if steps else None


def analyze(
    packets: list[Packet],
    mean_tolerance: float = 0.03,
    jitter: float = 0.25,
    within_share: float = 0.9,
    min_packets: int = 10,
) -> list[StreamResult]:
    """Checks every RTP stream of `packets` and returns one result per stream."""
    by_stream: dict[tuple, list[Packet]] = defaultdict(list)
    for p in packets:
        by_stream[p.stream].append(p)
    results = []
    for stream, group in by_stream.items():
        group.sort(key=lambda p: p.time)
        errors = []
        want = interval_ms(group) if len(group) >= 2 else None
        gaps = [(b.time - a.time) * 1000 for a, b in zip(group, group[1:])]
        if len(group) < min_packets:
            errors.append(f"only {len(group)} packets, at least {min_packets} needed")
        if want is None:
            errors.append(
                f"no packet interval from the RTP timestamps (payload type {group[0].payload_type})"
            )
        mean = statistics.fmean(gaps) if gaps else 0.0
        within = 0.0
        if want:
            if abs(mean - want) > mean_tolerance * want:
                errors.append(
                    f"mean inter arrival {mean:.2f} ms differs from the configured {want:.2f} ms "
                    f"by more than {mean_tolerance:.0%}"
                )
            within = sum(abs(g - want) <= jitter * want for g in gaps) / len(gaps)
            if within < within_share:
                errors.append(
                    f"only {within:.0%} of the gaps lie within {jitter:.0%} of {want:.2f} ms, "
                    f"{within_share:.0%} needed"
                )
        results.append(
            StreamResult(stream, len(group), want or 0.0, mean, within, errors)
        )
    return results


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("capture")
    ap.add_argument("--tshark", default="tshark")
    ap.add_argument("--mean-tolerance", type=float, default=0.03)
    ap.add_argument("--jitter", type=float, default=0.25)
    ap.add_argument("--within-share", type=float, default=0.9)
    ap.add_argument("--min-packets", type=int, default=10)
    ap.add_argument("--min-streams", type=int, default=2)
    args = ap.parse_args(argv)
    results = analyze(
        load(args.capture, args.tshark),
        args.mean_tolerance,
        args.jitter,
        args.within_share,
        args.min_packets,
    )
    failed = 0
    for r in results:
        src_ip, src_port, dst_ip, dst_port, ssrc = r.stream
        status = "ok" if not r.errors else "FAIL"
        print(
            f"rtp_check: {status} {src_ip}:{src_port} -> {dst_ip}:{dst_port} ssrc {ssrc} "
            f"{r.packets} packets, interval {r.interval_ms:.2f} ms, mean {r.mean_ms:.2f} ms, "
            f"within {r.within:.0%}"
        )
        for e in r.errors:
            print(f"rtp_check:   {e}", file=sys.stderr)
        failed += bool(r.errors)
    if len(results) < args.min_streams:
        print(
            f"rtp_check: {len(results)} RTP streams, at least {args.min_streams} needed",
            file=sys.stderr,
        )
        return 1
    if failed:
        print(
            f"rtp_check: {failed} of {len(results)} RTP streams fail", file=sys.stderr
        )
        return 1
    print(f"rtp_check: all {len(results)} RTP streams keep their packet interval")
    return 0


if __name__ == "__main__":
    sys.exit(main())
