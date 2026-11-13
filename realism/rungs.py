# SPDX-License-Identifier: GPL-3.0-only
"""The bytes per rung of an adaptive bit rate playback (RLM-22).

A player of HLS or DASH fetches segments of a ladder of rungs, each rung with its own bit
rate. A generator that sends the right number of segments at the right pace can still send
the wrong bytes, so the check reads the response sizes. The ``temporal.rungs`` labels of an
application give the request path of a segment (its first group is the rung), the media time
of one segment, and the bit rate of each rung. For every stream of requests (one keepalive
session, or the sessions of a playback that follow each other, see :func:`realism.exchanges.runs`)
with at least ``min_samples`` requests, the check requires that

* the body of every response lies within ``body_tolerance_bytes`` of ``rate * duration / 8``
  of its rung, which is one transport stream packet for MPEG-TS segments;
* the share of every rung in the bytes of the stream, counted as the octets of the response
  head and body, lies within ``share_tolerance`` (relative) of the share that the bit rates of
  the requested rungs give;
* the bit rate of the whole stream, those octets over the media time of its requests, lies
  within ``rate_tolerance`` (relative) of the mean bit rate of the requested rungs;
* with ``walk: up``, the rungs rise and visit the whole ladder.

These are the checks of ``modules/hls/fixtures/hls_check.py`` (MOD-15), which a later package
replaces with these labels. The share and the rate follow from the bytes alone, so a stream
that carries the right bytes at the wrong pace passes here and fails the packet gap target.
"""

from __future__ import annotations

import re
from collections.abc import Sequence
from typing import Any

from realism import exchanges
from realism.exchanges import SessionExchanges
from realism.labels import RungTarget

STATUS_PASS = "pass"
STATUS_FAIL = "fail"
STATUS_INSUFFICIENT = "insufficient_data"
MAX_STREAMS_LISTED = 100


def judge(
    target: RungTarget, read: Sequence[SessionExchanges], minimum: int, label: str
) -> dict[str, Any]:
    """Judge the streams of one application and return the entry of the report.

    ``minimum`` is the section value of ``min_samples``, which the target replaces.
    ``label`` names the target in a reason.
    """
    needed = target.min_samples or minimum
    entry: dict[str, Any] = {
        "application": target.application,
        "path": target.path,
        "duration": target.duration,
        "bits_per_second": list(target.rates),
        "min_samples": needed,
        "expected_streams": None if target.streams is None else target.streams.to_dict(),
        "requests": 0,
        "status": STATUS_INSUFFICIENT,
        "reason": None,
        "streams": None,
        "playbacks": [],
    }
    error = exchanges.first_error(read)
    if error is not None:
        entry["status"] = STATUS_FAIL
        entry["reason"] = f"{label}: the {error}"
        return entry
    pattern = re.compile(target.path)
    runs = exchanges.runs(read, pattern, target.join)
    entry["requests"] = sum(len(run.exchanges) for run in runs)
    judged = [run for run in runs if len(run.exchanges) >= needed]
    if not judged:
        entry["reason"] = (
            f"{label}: insufficient data, no stream has the {needed} requests that are needed "
            "(temporal.min_samples), so the bytes are not judged"
        )
        return entry
    results = [_judge_stream(target, run) for run in judged]
    failed = [result for result in results if result["failures"]]
    entry["streams"] = {
        "judged": len(judged),
        "failed": len(failed),
        "too_short": len(runs) - len(judged),
    }
    shown = sorted(results, key=lambda result: not result["failures"])[:MAX_STREAMS_LISTED]
    entry["playbacks"] = sorted(shown, key=lambda result: result["name"])
    reasons: list[str] = []
    if failed:
        first = failed[0]
        reasons.append(
            f"{label}: {len(failed)} of {len(judged)} streams carry the wrong bytes; the first "
            f"is the {first['name']}: {first['failures'][0]}"
            + (
                f" (and {len(first['failures']) - 1} more in it)"
                if len(first["failures"]) > 1
                else ""
            )
        )
    if target.streams is not None and not target.streams.holds(len(judged)):
        reasons.append(
            f"{label}: {len(judged)} streams have at least {needed} requests, and the labels "
            f"expect {target.streams.text()}"
        )
    entry["status"] = STATUS_FAIL if reasons else STATUS_PASS
    entry["reason"] = "; ".join(reasons) or None
    return entry


def _judge_stream(target: RungTarget, run: exchanges.Run) -> dict[str, Any]:
    rates = target.rates
    pattern = re.compile(target.path)
    out: dict[str, Any] = {
        "name": run.name,
        "requests": len(run.exchanges),
        "failures": [],
    }
    fail = out["failures"].append
    rungs = []
    for exchange in run.exchanges:
        match = pattern.search(exchange.target)
        assert match is not None  # runs() keeps only the matching exchanges
        try:
            rung = int(match.group(1))
        except (TypeError, ValueError):
            fail(f"{exchange.target}: the first group of the path is not a rung number")
            return out
        if rung >= len(rates):
            fail(f"{exchange.target} names rung {rung} of a ladder with {len(rates)} rungs")
            return out
        rungs.append(rung)
    if target.walk_up and (rungs != sorted(rungs) or set(rungs) != set(range(len(rates)))):
        fail(f"the rungs {sorted(set(rungs))} do not walk up the whole ladder")
    wire = [0.0] * len(rates)
    expect = [0.0] * len(rates)
    for exchange, rung in zip(run.exchanges, rungs, strict=True):
        want = rates[rung] * target.duration / 8
        if abs(exchange.body_bytes - want) > target.body_tolerance:
            fail(f"{exchange.target}: body of {exchange.body_bytes} octets against {want:.0f}")
        wire[rung] += exchange.wire_bytes
        expect[rung] += want
    total, want_total = sum(wire), sum(expect)
    shares: dict[str, float] = {}
    for rung, (got_bytes, want_bytes) in enumerate(zip(wire, expect, strict=True)):
        if want_bytes == 0:
            continue
        got, want_share = got_bytes / total, want_bytes / want_total
        shares[str(rung)] = round(got, 4)
        if abs(got - want_share) > target.share_tolerance * want_share:
            fail(f"rung {rung} carries {got:.1%} of the bytes against {want_share:.1%}")
    seconds = len(run.exchanges) * target.duration
    rate, want_rate = total * 8 / seconds, want_total * 8 / seconds
    out.update(
        rung_share=shares,
        bits_per_second=round(rate),
        configured_bits_per_second=round(want_rate),
    )
    if abs(rate - want_rate) > target.rate_tolerance * want_rate:
        fail(f"bit rate {rate:.0f} against the configured {want_rate:.0f}")
    return out
