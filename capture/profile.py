# SPDX-License-Identifier: GPL-3.0-only
"""Profile extraction (CAP-04): an application mix and temporal models from sessions.

The input is the session document of ``python3 -m capture sessions`` (payloads are not
needed) and the output is a ``profile`` object that fits the ``profile`` of a test
configuration (``contracts/schemas/test-config.schema.json``): the mix with a weight per
application, an arrival model for the whole run, and an arrival model and size models
for each mix entry. A report beside it holds the evidence of every number.

Applications come from the server port, because the compiler reads no protocol. A table
of well known ports gives the default names, and ``--app`` rules replace it. A session
that no rule names is counted as unclassified and stays out of the mix, as the passive
data connections of FTP stay out of the realism labels.

The models follow the Swing method: each distribution is fitted from the observed
samples. Five families are candidates, the ones that the contract names. The one with the
smallest Kolmogorov Smirnov (KS) distance to the sample wins, and a sample whose values
all lie within one percent of their median is a constant.
"""

from __future__ import annotations

import math
import re
import statistics
from collections import Counter, defaultdict
from collections.abc import Callable, Iterable, Sequence
from itertools import pairwise
from typing import Any

from .scrub import check_record

PROFILER = "capture profile"
PROFILER_VERSION = "1.0.0"
SCHEMA_VERSION = "1.0"

# Server ports of the protocols of the application modules, by transport. A rule of the
# user takes precedence, and ``defaults=False`` turns the table off.
DEFAULT_APPS: dict[tuple[str, int], str] = {
    ("tcp", 21): "ftp",
    ("tcp", 25): "smtp",
    ("tcp", 587): "smtp",
    ("tcp", 53): "dns",
    ("udp", 53): "dns",
    ("tcp", 80): "http",
    ("tcp", 8080): "http",
    ("tcp", 110): "pop3",
    ("tcp", 143): "imap",
    ("tcp", 443): "https",
    ("tcp", 8443): "https",
}

BASES = ("sessions", "bytes")
DEFAULT_MIN_SAMPLES = 30  # the realism temporal check also needs 30 observations
CONSTANT_SPREAD = 0.01  # a sample within 1% of its median is a constant
_NAME = re.compile(r"^[a-z][a-z0-9_]*(/[a-z][a-z0-9_]*)?$")
_RULE = re.compile(r"^(?:(tcp|udp):)?(\d{1,5})(?:-(\d{1,5}))?=(.+)$")
_UNCLASSIFIED_LISTED = 20
_FAMILIES = ("exponential", "uniform", "lognormal", "pareto")


class ProfileError(Exception):
    """The input or an option cannot produce a profile."""


Rules = list[tuple[str | None, int, int, str]]


def parse_rules(specs: Iterable[str]) -> Rules:
    """Parse ``[tcp:|udp:]PORT[-PORT]=NAME`` rules. The name is a scenario reference."""
    rules: Rules = []
    for spec in specs:
        m = _RULE.match(spec)
        if m is None:
            raise ProfileError(f"--app {spec!r} is not [tcp:|udp:]PORT[-PORT]=NAME")
        proto, lo, hi, name = m.group(1), int(m.group(2)), m.group(3), m.group(4)
        high = int(hi) if hi is not None else lo
        if not 0 < lo <= high <= 65535:
            raise ProfileError(f"--app {spec!r}: the port range must lie within 1 to 65535")
        if not _NAME.match(name):
            raise ProfileError(
                f"--app {spec!r}: the name must be an application or a module/scenario "
                "reference of lowercase letters, digits, and underscores"
            )
        rules.append((proto, lo, high, name))
    return rules


def classify(session: dict[str, Any], rules: Rules, defaults: bool) -> str | None:
    """Return the application of a session, or None.

    The server port decides. When the capture did not show who opened the session, the
    client port is tried as well, because the guess of the roles may be wrong."""
    ports = [session["server"]["port"]]
    if not session["roles"]["client_known"]:
        ports.append(session["client"]["port"])
    proto = session["protocol"]
    for port in ports:
        for rule_proto, lo, hi, name in rules:
            if rule_proto in (None, proto) and lo <= port <= hi:
                return name
        if defaults and (proto, port) in DEFAULT_APPS:
            return DEFAULT_APPS[(proto, port)]
    return None


def session_bytes(session: dict[str, Any]) -> tuple[int, int]:
    """Return the distinct payload bytes of both directions, client to server first.

    A TCP stream counts each byte once, so a retransmission does not weigh a session."""
    out = []
    for key in ("client_to_server", "server_to_client"):
        d = session[key]
        out.append(int(d["stream_bytes"] if "stream_bytes" in d else d["payload_bytes"]))
    return out[0], out[1]


def size_units(session: dict[str, Any]) -> dict[str, list[int]]:
    """Return the payload units of a session by direction, empty ones left out.

    A UDP datagram is one unit. The compiler does not split a TCP stream into messages,
    so a whole direction of a TCP session is one unit, as the contract allows."""
    units: dict[str, list[int]] = {"request": [], "response": []}
    if session["protocol"] == "udp":
        for turn in session["turns"]:
            key = "request" if turn["direction"] == "client_to_server" else "response"
            units[key] += [n for n in turn.get("datagram_sizes", []) if n > 0]
    else:
        up, down = session_bytes(session)
        units["request"] += [up] if up > 0 else []
        units["response"] += [down] if down > 0 else []
    return units


def _round(value: float) -> float:
    return float(f"{value:.6g}")


def _ks(sample: Sequence[float], cdf: Callable[[float], float]) -> float:
    n = len(sample)
    d = 0.0
    for i, x in enumerate(sample):
        f = cdf(x)
        d = max(d, f - i / n, (i + 1) / n - f)
    return d


def _phi(z: float) -> float:
    return 0.5 * (1.0 + math.erf(z / math.sqrt(2.0)))


def fit_distribution(
    values: Sequence[float],
) -> tuple[str, dict[str, float], float, dict[str, float]]:
    """Fit the five families of the contract to a sample of positive or zero values.

    Returns the winning family, its parameters in the units of the sample, its KS
    distance, and the distance of every family that could be fitted. The parameters are
    those of the contract with the unit suffix left off, so the caller renames them."""
    xs = sorted(values)
    n = len(xs)
    med = statistics.median(xs)
    spread = xs[-1] - xs[0]
    if spread <= CONSTANT_SPREAD * med:
        dev = max(abs(x - med) for x in xs)
        return "constant", {"value": med, "tolerance": dev}, 0.0, {}
    scores: dict[str, tuple[float, dict[str, float]]] = {}
    mean = sum(xs) / n
    scores["exponential"] = (_ks(xs, lambda x: 1.0 - math.exp(-x / mean)), {"mean": mean})
    lo, hi = xs[0], xs[-1]
    scores["uniform"] = (_ks(xs, lambda x: (x - lo) / (hi - lo)), {"min": lo, "max": hi})
    if lo > 0:
        logs = [math.log(x) for x in xs]
        mu = sum(logs) / n
        sigma = math.sqrt(sum((v - mu) ** 2 for v in logs) / n)
        if sigma > 0:
            scores["lognormal"] = (
                _ks(xs, lambda x: _phi((math.log(x) - mu) / sigma)),
                {"mu": mu, "sigma": sigma},
            )
        tail = sum(v - math.log(lo) for v in logs)
        if tail > 0:
            alpha = n / tail
            scores["pareto"] = (
                _ks(xs, lambda x: 1.0 - (lo / x) ** alpha),
                {"alpha": alpha, "scale": lo},
            )
    order = {name: i for i, name in enumerate(_FAMILIES)}
    best = min(scores, key=lambda k: (round(scores[k][0], 12), order[k]))
    return best, scores[best][1], scores[best][0], {k: _round(v[0]) for k, v in scores.items()}


def arrival_model(
    gaps_ms: Sequence[float],
) -> tuple[dict[str, Any], float, dict[str, float]]:
    """Fit the gaps between session starts and write the contract's ``arrival`` object."""
    family, p, d, others = fit_distribution(gaps_ms)
    spec: dict[str, Any]
    if family == "constant":
        spec = {"dist": "constant", "ms": p["value"], "tolerance_ms": p["tolerance"]}
    elif family == "exponential":
        spec = {"dist": "exponential", "mean_ms": p["mean"]}
    elif family == "uniform":
        spec = {"dist": "uniform", "min_ms": p["min"], "max_ms": p["max"]}
    elif family == "lognormal":
        # the contract's default unit is seconds, so shift mu by ln 1000
        spec = {
            "dist": "lognormal",
            "mu": p["mu"] - math.log(1000.0),
            "sigma": p["sigma"],
            "unit": "s",
        }
    else:
        spec = {"dist": "pareto", "alpha": p["alpha"], "scale_ms": p["scale"]}
    return _rounded(spec), d, others


def size_model(sizes: Sequence[float]) -> tuple[dict[str, Any], float, dict[str, float]]:
    """Fit payload sizes in bytes and write the contract's ``size_distribution`` object."""
    family, p, d, others = fit_distribution(sizes)
    spec: dict[str, Any] = {"dist": family, **p}
    return _rounded(spec), d, others


def _rounded(spec: dict[str, Any]) -> dict[str, Any]:
    return {k: (_round(v) if isinstance(v, float) else v) for k, v in spec.items()}


def _fit_block(
    samples: Sequence[float],
    fitter: Callable[[Sequence[float]], tuple[dict[str, Any], float, dict[str, float]]],
    min_samples: int,
) -> tuple[dict[str, Any] | None, dict[str, Any]]:
    """Return the model, or None, and its evidence for the report."""
    if len(samples) < min_samples:
        return None, {"samples": len(samples), "model": None, "reason": "insufficient_data"}
    if max(samples) <= 0:
        return None, {"samples": len(samples), "model": None, "reason": "all_zero"}
    spec, d, others = fitter(samples)
    return spec, {
        "samples": len(samples),
        "model": spec,
        "ks_statistic": _round(d),
        "candidates": others,
    }


def _share(part: float, whole: float) -> float:
    return round(100.0 * part / whole, 4) if whole else 0.0


def build_profile(
    doc: dict[str, Any],
    *,
    basis: str = "sessions",
    app_rules: Sequence[str] = (),
    defaults: bool = True,
    min_samples: int = DEFAULT_MIN_SAMPLES,
) -> dict[str, Any]:
    """Return the profile document of a session document.

    The document holds the ``profile`` for a test configuration, the evidence per
    application, the sessions that no rule named, and the provenance of the input."""
    if basis not in BASES:
        raise ProfileError(f"basis must be one of {', '.join(BASES)}")
    if min_samples < 2:
        raise ProfileError("min samples must be at least 2")
    if not isinstance(doc, dict) or not str(doc.get("schema_version", "")).startswith("1."):
        raise ProfileError("the input is not a session document of schema version 1.x")
    rules = parse_rules(app_rules)
    notes: list[str] = []

    counted: dict[str, list[dict[str, Any]]] = defaultdict(list)
    app_bytes: Counter[str] = Counter()
    app_up: Counter[str] = Counter()
    app_down: Counter[str] = Counter()
    units: dict[str, dict[str, list[int]]] = defaultdict(lambda: {"request": [], "response": []})
    lost: Counter[tuple[str, int]] = Counter()
    lost_bytes = 0
    lost_sessions = 0
    continued = 0
    for s in doc["sessions"]:
        up, down = session_bytes(s)
        app = classify(s, rules, defaults)
        if app is None:
            lost_sessions += 1
            lost_bytes += up + down
            lost[(s["protocol"], s["server"]["port"])] += 1
            continue
        app_bytes[app] += up + down
        app_up[app] += up
        app_down[app] += down
        for key, vals in size_units(s).items():
            units[app][key] += vals
        if s["continues"] is not None:
            continued += 1  # the rest of a UDP flow that an idle gap split: no new session
        else:
            counted[app].append(s)
    if continued:
        notes.append(
            f"{continued} UDP sessions continue an earlier flow after an idle gap. Their "
            "bytes and datagrams count, but they are not new sessions of the mix or arrivals."
        )

    names = sorted(set(counted) | set(app_bytes))
    total_sessions = sum(len(v) for v in counted.values())
    total_bytes = sum(app_bytes.values())
    if not names:
        raise ProfileError(
            "no session matched an application: name server ports with --app, "
            "for example --app 8443=https"
        )

    mix: list[dict[str, Any]] = []
    apps: list[dict[str, Any]] = []
    for name in names:
        starts = sorted(s["start_ns"] for s in counted[name])
        gaps = [(b - a) / 1e6 for a, b in pairwise(starts)]
        arrival, arrival_ev = _fit_block(gaps, arrival_model, min_samples)
        sizes: dict[str, Any] = {}
        size_ev: dict[str, Any] = {}
        for key in ("request", "response"):
            model, ev = _fit_block(units[name][key], size_model, min_samples)
            size_ev[key] = ev
            if model is not None:
                sizes[key] = model
        sess_share = _share(len(counted[name]), total_sessions)
        byte_share = _share(app_bytes[name], total_bytes)
        weight = sess_share if basis == "sessions" else byte_share
        entry: dict[str, Any] = {"scenario": name, "weight": weight}
        if arrival is not None:
            entry["arrival"] = arrival
        if sizes:
            entry["size"] = sizes
        if weight > 0:
            mix.append(entry)
        else:
            notes.append(f"{name} has no weight on the {basis} basis and is left out of the mix.")
        apps.append(
            {
                "name": name,
                "sessions": len(counted[name]),
                "sessions_share": sess_share,
                "payload_bytes": app_bytes[name],
                "payload_bytes_share": byte_share,
                "client_to_server_bytes": app_up[name],
                "server_to_client_bytes": app_down[name],
                "arrival": arrival_ev,
                "size": size_ev,
            }
        )
    mix.sort(key=lambda e: (-e["weight"], e["scenario"]))
    apps.sort(key=lambda a: (-a["sessions"], a["name"]))

    all_starts = sorted(s["start_ns"] for v in counted.values() for s in v)
    overall, overall_ev = _fit_block(
        [(b - a) / 1e6 for a, b in pairwise(all_starts)], arrival_model, min_samples
    )
    profile: dict[str, Any] = {"basis": basis}
    if overall is not None:
        profile["arrival"] = overall
    profile["mix"] = mix

    record, problems = check_record(doc)
    scrub = _scrub_state(doc, record, problems, notes)
    src = doc.get("source", {})
    return {
        "schema_version": SCHEMA_VERSION,
        "tool": PROFILER,
        "version": PROFILER_VERSION,
        "source": {
            "file": src.get("file"),
            "sha256": src.get("sha256"),
            "sessions": len(doc["sessions"]),
        },
        "scrub": scrub,
        "parameters": {
            "basis": basis,
            "min_samples": min_samples,
            "default_apps": defaults,
            "app_rules": list(app_rules),
        },
        "profile": profile,
        "overall": {
            "sessions": total_sessions,
            "payload_bytes": total_bytes,
            "arrival": overall_ev,
        },
        "applications": apps,
        "unclassified": {
            "sessions": lost_sessions,
            "payload_bytes": lost_bytes,
            "server_ports": [
                {"protocol": p, "port": port, "sessions": n}
                for (p, port), n in sorted(lost.items(), key=lambda kv: (-kv[1], kv[0]))[
                    :_UNCLASSIFIED_LISTED
                ]
            ],
        },
        "notes": notes,
    }


def _scrub_state(
    doc: dict[str, Any],
    record: dict[str, Any] | None,
    problems: list[str],
    notes: list[str],
) -> dict[str, Any]:
    """Say whether the input carried an intact scrub record, and what it dropped.

    The profile holds only counts, times, and sizes, so it carries no payload content
    either way. A scrub that dropped sessions changes the mix, so the profile says so."""
    if "scrub" not in doc:
        return {"state": "none"}
    if record is None:
        notes.append("the input has a scrub record that is not intact: " + "; ".join(problems))
        return {"state": "invalid", "problems": problems}
    dropped = int(record["options"]["dropped_sessions"])
    if dropped:
        notes.append(
            f"the scrub dropped {dropped} sessions (ports {record['options']['dropped_ports']}), "
            "so the mix leaves them out"
        )
    return {
        "state": "intact",
        "date": record["date"],
        "capture_sha256": record["capture_sha256"],
        "input_sha256": record["input_sha256"],
        "content_sha256": record["content_sha256"],
        "dropped_sessions": dropped,
    }
