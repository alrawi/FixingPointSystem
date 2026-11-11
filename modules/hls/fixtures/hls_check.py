#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Checks the segment cadence and the byte share of the HLS playbacks in a capture.

The acceptance of MOD-15 asks that the temporal check accepts the segment fetch
cadence and that the byte share matches the configured bit rate within 5%. The
temporal check of the realism runner (realism/temporal.py) compares the gaps
between the starts of sessions, which is the cadence of the scenario
`vod_segments`, where each segment has its own connection. It cannot see the
gaps between the requests inside one keepalive connection, which is the cadence
of `vod_ladder` and `live_refresh`, and it does not read bytes. This check does
both, from the capture alone with the pure Python reader of the runner.

It reassembles the two directions of every TCP session that talks to a port of
the module, reads the HTTP requests and responses (RFC 9112) out of the
streams, and groups the requests for segments into playbacks. A playback is one
keepalive session, or the run of segment sessions on the segment port whose
starts are less than two segment durations apart. For each playback it requires

* a mean gap between consecutive segment requests within `--mean-tolerance` of
  the segment duration, and at least `--within-share` of the gaps within
  `--jitter` of it, so a pacing that drifts or bursts fails;
* the body of every segment within one transport stream packet (188 octets) of
  the bit rate of its rung times the segment duration;
* the share of every rung in the bytes of the playback, counted on the wire
  with the HTTP headers, within `--byte-tolerance` of the share that the
  bit rates of the requested rungs give, and the bit rate of the whole playback
  within `--byte-tolerance` of the mean bit rate of those rungs.

The ladder and the segment duration come from `--params`, which is the file
that `capture.sh` records the capture with.

    python3 hls_check.py CAPTURE [--params params.json] [--mean-tolerance 0.05]
                         [--jitter 0.10] [--within-share 0.9] [--byte-tolerance 0.05]
                         [--min-segments 4] [--output FILE]
"""

from __future__ import annotations

import argparse
import bisect
import json
import re
import statistics
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from realism.capture import decode_segment, read_frames  # noqa: E402

TS_PACKET = 188
REQUEST_LINE = re.compile(rb"^GET (\S+) HTTP/1\.1\r\n")
SEGMENT_PATH = re.compile(r"/v(\d+)/seg(\d+)\.ts$")


class CheckError(Exception):
    """The capture cannot be read as the module writes it."""


@dataclass
class Exchange:
    """One request with its response, from one session."""

    time: float
    target: str
    wire_bytes: int
    body_bytes: int


@dataclass
class Playback:
    name: str
    exchanges: list[Exchange] = field(default_factory=list)

    def segments(self) -> list[tuple[Exchange, int]]:
        out = []
        for e in self.exchanges:
            m = SEGMENT_PATH.search(e.target)
            if m:
                out.append((e, int(m.group(1))))
        return out


def parse_ladder(text: str) -> list[int]:
    rates = []
    for item in text.split(","):
        rate, _, _ = item.strip().partition("@")
        rates.append(int(rate))
    return rates


def _stream(segments: list[tuple[int, float, bytes]]) -> tuple[bytes, list[int], list[float]]:
    """Joins the segments of one direction in sequence order."""
    seen: dict[int, tuple[float, bytes]] = {}
    base = None
    for seq, ts, payload in segments:
        if base is None:
            base = seq
        rel = (seq - base) & 0xFFFFFFFF
        seen.setdefault(rel, (ts, payload))
    data = bytearray()
    offsets: list[int] = []
    times: list[float] = []
    for rel in sorted(seen):
        ts, payload = seen[rel]
        if rel != len(data):
            raise CheckError(f"the stream has a gap or an overlap at octet {len(data)}")
        offsets.append(rel)
        times.append(ts)
        data += payload
    return bytes(data), offsets, times


def read_sessions(path: Path, ports: set[int]) -> dict[tuple, dict]:
    """Reassembles the TCP sessions that have a server on one of `ports`."""
    sessions: dict[tuple, dict] = {}
    _, frames = read_frames(path)
    for frame in frames:
        seg = decode_segment(frame)
        if seg is None or seg.packet.transport != "tcp" or not seg.payload:
            continue
        p = seg.packet
        if p.dst_port in ports:
            key, direction, port = (p.src_ip, p.src_port, p.dst_ip, p.dst_port), "c2s", p.dst_port
        elif p.src_port in ports:
            key, direction, port = (p.dst_ip, p.dst_port, p.src_ip, p.src_port), "s2c", p.src_port
        else:
            continue
        s = sessions.setdefault(key, {"port": port, "c2s": [], "s2c": []})
        s[direction].append((seg.seq, p.timestamp, seg.payload))
    return sessions


def read_exchanges(session: dict) -> list[Exchange]:
    c2s, c_offsets, c_times = _stream(session["c2s"])
    s2c, _, _ = _stream(session["s2c"])
    requests = []
    at = 0
    while at < len(c2s):
        end = c2s.find(b"\r\n\r\n", at)
        if end < 0:
            raise CheckError("a request is cut off")
        m = REQUEST_LINE.match(c2s[at : end + 4])
        if not m:
            raise CheckError(f"not a GET request: {c2s[at : at + 40]!r}")
        i = bisect.bisect_right(c_offsets, at) - 1
        requests.append((c_times[i], m.group(1).decode()))
        at = end + 4
    out = []
    at = 0
    for time, target in requests:
        end = s2c.find(b"\r\n\r\n", at)
        if end < 0:
            raise CheckError(f"the response to {target} is missing")
        head = s2c[at:end].decode("latin-1")
        m = re.search(r"(?im)^content-length:\s*(\d+)\s*$", head)
        if not m:
            raise CheckError(f"the response to {target} has no Content-Length")
        body = int(m.group(1))
        stop = end + 4 + body
        if stop > len(s2c):
            raise CheckError(f"the response to {target} is cut off")
        out.append(Exchange(time, target, stop - at, body))
        at = stop
    if at != len(s2c):
        raise CheckError("the server wrote bytes after the last response")
    return out


def playbacks(sessions: dict[tuple, dict], ports: dict[str, int], segment_ms: int) -> list[Playback]:
    result: list[Playback] = []
    segment_port = ports["segments"]
    names = {v: k for k, v in ports.items()}
    per_segment: list[Exchange] = []
    for key in sorted(sessions, key=lambda k: min(t for _, t, _ in sessions[k]["c2s"])):
        s = sessions[key]
        exchanges = read_exchanges(s)
        if s["port"] == segment_port:
            per_segment.extend(exchanges)
            continue
        if s["port"] == ports["vod_segments"]:
            # The playlist connection of vod_segments carries no segment. Its segments
            # are on the connections of the segment port.
            continue
        result.append(Playback(f"{names[s['port']]}:{key[1]}", exchanges))
    per_segment.sort(key=lambda e: e.time)
    run: Playback | None = None
    for e in per_segment:
        if run is None or e.time - run.exchanges[-1].time > 2 * segment_ms / 1000:
            run = Playback(f"vod_segments:{len(result)}")
            result.append(run)
        run.exchanges.append(e)
    return result


def check_playback(pb: Playback, rates: list[int], a: argparse.Namespace) -> dict:
    segs = pb.segments()
    out: dict = {"name": pb.name, "segments": len(segs), "failures": []}
    fail = out["failures"].append
    if len(segs) < a.min_segments:
        fail(f"{len(segs)} segments, fewer than {a.min_segments}")
        return out
    want_gap = a.segment_ms / 1000
    times = [e.time for e, _ in segs]
    gaps = [b - x for x, b in zip(times, times[1:])]
    mean = statistics.fmean(gaps)
    within = sum(abs(g - want_gap) <= a.jitter * want_gap for g in gaps) / len(gaps)
    out.update(mean_gap_ms=round(mean * 1000, 3), within_share=round(within, 3))
    if abs(mean - want_gap) > a.mean_tolerance * want_gap:
        fail(f"mean gap {mean * 1000:.1f} ms against {a.segment_ms} ms")
    if within < a.within_share:
        fail(f"{within:.0%} of the gaps within {a.jitter:.0%} of {a.segment_ms} ms")
    wire = [0.0] * len(rates)
    expect = [0.0] * len(rates)
    if pb.name.startswith("vod_"):
        walk = [rung for _, rung in segs]
        if walk != sorted(walk) or set(walk) != set(range(len(rates))):
            fail(f"the rungs {sorted(set(walk))} do not walk up the whole ladder")
    for e, rung in segs:
        if rung >= len(rates):
            fail(f"{e.target} names rung {rung} of a ladder with {len(rates)} rungs")
            return out
        want = rates[rung] * a.segment_ms / 8000
        if abs(e.body_bytes - want) > TS_PACKET:
            fail(f"{e.target}: body of {e.body_bytes} octets against {want:.0f}")
        wire[rung] += e.wire_bytes
        expect[rung] += rates[rung] * a.segment_ms / 8000
    total, want_total = sum(wire), sum(expect)
    shares = {}
    for i, (w, x) in enumerate(zip(wire, expect)):
        if x == 0:
            continue
        got, want_share = w / total, x / want_total
        shares[i] = round(got, 4)
        if abs(got - want_share) > a.byte_tolerance * want_share:
            fail(f"rung {i} carries {got:.1%} of the bytes against {want_share:.1%}")
    seconds = len(segs) * a.segment_ms / 1000
    rate = total * 8 / seconds
    want_rate = want_total * 8 / seconds
    out.update(rung_share=shares, bits_per_second=round(rate), configured_bits_per_second=round(want_rate))
    if abs(rate - want_rate) > a.byte_tolerance * want_rate:
        fail(f"playback bit rate {rate:.0f} against the configured {want_rate:.0f}")
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("capture", type=Path)
    ap.add_argument("--params", type=Path, default=Path(__file__).with_name("params.json"))
    ap.add_argument("--mean-tolerance", type=float, default=0.05)
    ap.add_argument("--jitter", type=float, default=0.10)
    ap.add_argument("--within-share", type=float, default=0.9)
    ap.add_argument("--byte-tolerance", type=float, default=0.05)
    ap.add_argument("--min-segments", type=int, default=4)
    ap.add_argument("--output", type=Path)
    a = ap.parse_args(argv)
    params = json.loads(a.params.read_text())
    a.segment_ms = params["segment_ms"]
    rates = parse_ladder(params["ladder"])
    ports = params["ports"]
    try:
        sessions = read_sessions(a.capture, set(ports.values()))
        if not sessions:
            raise CheckError("the capture has no session on a port of the module")
        found = playbacks(sessions, ports, a.segment_ms)
        want = params["sessions"]["vod_ladder"] + params["sessions"]["vod_segments"] + params["sessions"]["live_refresh"]
        if len(found) != want:
            raise CheckError(f"the capture holds {len(found)} playbacks and the peers ran {want}")
        results = [check_playback(p, rates, a) for p in found]
    except CheckError as e:
        print(f"hls_check: {e}", file=sys.stderr)
        return 1
    failed = 0
    for r in results:
        status = "FAIL" if r["failures"] else "ok"
        print(
            f"{status} {r['name']}: {r['segments']} segments, mean gap {r.get('mean_gap_ms')} ms, "
            f"{r.get('bits_per_second')} bit/s against {r.get('configured_bits_per_second')}"
        )
        for f in r["failures"]:
            print(f"    {f}")
        failed += bool(r["failures"])
    if a.output:
        a.output.write_text(json.dumps({"playbacks": results}, indent=2) + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
