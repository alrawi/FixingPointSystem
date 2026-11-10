# SPDX-License-Identifier: GPL-3.0-only
"""Generate a module directory of scenarios from extracted sessions (CAP-02).

Each session becomes one flow with a client program and a server program. A turn becomes
a ``send`` on the side that sent it and one or more ``expect`` actions on the other side.
Field detection (``fields.py``) replaces dynamic values by template expressions, and
every generated template is rendered with the reference renderer of ``templates.py`` to
check that it keeps the byte count of the captured payload.
"""

from __future__ import annotations

import base64
import binascii
import json
import re
from dataclasses import dataclass, field
from itertools import pairwise
from pathlib import Path
from typing import Any

from .fields import (
    Analysis,
    Config,
    Field,
    Span,
    VarAlloc,
    analyze_framing,
    detect_fields,
)
from .templates import escape_literal, render

IDENT = re.compile(r"^[a-z][a-z0-9_]{0,62}$")
ENGINE_API = ">=0.2, <0.3"
_DELIMITERS = (b"\r\n\r\n", b"\r\n", b"\n")
_UP, _DOWN = 0, 1
_DIRS = ("up", "down")


class ScenarioError(Exception):
    """The input cannot become a scenario. The message says why."""


@dataclass
class Unit:
    """One payload that a side sends: a TCP turn or a UDP datagram."""

    direction: int
    data: bytes
    start_ns: int
    end_ns: int
    flow: int  # index of the flow in the scenario
    udp: bool
    gap_before: int = 0
    first_in_turn: bool = True


@dataclass
class FlowInfo:
    session: dict[str, Any]
    port_var: str


@dataclass
class Counts:
    """What the generator replaced, for the report."""

    lengths: int = 0
    fills: int = 0
    frames: int = 0
    http_dates: int = 0
    epochs: int = 0
    identifiers: int = 0
    cookies: int = 0
    captures: int = 0
    static_vars: int = 0

    def as_dict(self) -> dict[str, int]:
        return dict(self.__dict__)


@dataclass
class Scenario:
    name: str
    text: str
    expected_bytes: int
    counts: Counts
    warnings: list[str] = field(default_factory=list)
    sessions: list[int] = field(default_factory=list)


# -- input ----------------------------------------------------------------------------


def load_sessions(path: Path) -> dict[str, Any]:
    """Read and check a session document written by ``python3 -m capture sessions``."""
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise ScenarioError(f"cannot read {path}: {exc}") from exc
    if not isinstance(doc, dict) or not str(doc.get("schema_version", "")).startswith("1."):
        raise ScenarioError(f"{path} is not a session document of schema version 1.x")
    if not doc.get("parameters", {}).get("payloads"):
        raise ScenarioError(
            f"{path} has no payload bytes; run `python3 -m capture sessions --payloads` first"
        )
    return doc


def select_sessions(doc: dict[str, Any], ids: list[int] | None) -> list[dict[str, Any]]:
    """Return the sessions named by ``ids``, or all of them, in start order."""
    by_id = {s["id"]: s for s in doc["sessions"]}
    if ids is None:
        return list(doc["sessions"])
    missing = [i for i in ids if i not in by_id]
    if missing:
        raise ScenarioError(f"no session with id {', '.join(map(str, missing))}")
    return [by_id[i] for i in dict.fromkeys(ids)]


def _units(session: dict[str, Any], flow: int, warnings: list[str]) -> list[Unit]:
    udp = session["protocol"] == "udp"
    out: list[Unit] = []
    for n, turn in enumerate(session["turns"]):
        direction = _UP if turn["direction"] == "client_to_server" else _DOWN
        try:
            if udp:
                blobs = [base64.b64decode(b, validate=True) for b in turn["datagrams_b64"]]
            else:
                blobs = [base64.b64decode(turn["data_b64"], validate=True)]
        except (KeyError, binascii.Error) as exc:
            raise ScenarioError(
                f"session {session['id']} turn {n} has no usable payload: {exc}"
            ) from exc
        if turn.get("gap_before"):
            warnings.append(
                f"session {session['id']} turn {n} follows a gap of {turn['gap_before']} bytes; "
                "the replay is shorter than the capture"
            )
        for k, blob in enumerate(blobs):
            if blob:
                out.append(
                    Unit(
                        direction, blob, turn["start_ns"], turn["end_ns"], flow, udp,
                        turn.get("gap_before", 0) if k == 0 else 0, k == 0,
                    )
                )  # fmt: skip
    return out


# -- YAML output ----------------------------------------------------------------------


def _flow(value: Any) -> str:
    """YAML flow style of a scalar, a list, or a dict, with every string quoted."""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, list):
        return "[" + ", ".join(_flow(v) for v in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{k}: {_flow(v)}" for k, v in value.items()) + " }"
    raise TypeError(f"cannot write {value!r}")


def _actions(lines: list[str], key: str, actions: list[dict[str, Any]], pad: str) -> None:
    lines.append(f"{pad}{key}:")
    for action in actions:
        ((name, body),) = action.items()
        lines.append(f"{pad}  - {name}: {_flow(body)}")


def scenario_yaml(
    name: str,
    description: str,
    vars_: dict[str, int | str],
    flows: list[dict[str, Any]],
) -> str:
    """Write a scenario document. Strings are always double quoted, so no value is
    reinterpreted by a YAML reader."""
    lines = [f"scenario: {name}", f"description: {json.dumps(description)}"]
    if vars_:
        lines.append("vars:")
        lines += [f"  {k}: {_flow(v)}" for k, v in vars_.items()]
    lines.append("flows:")
    for fl in flows:
        lines.append(f"  - id: {fl['id']}")
        lines.append(f"    module: {fl['module']}")
        if fl.get("l4"):
            lines.append(f"    l4: {fl['l4']}")
        if fl.get("after"):
            lines.append(f"    after: {fl['after']}")
        _actions(lines, "client", fl["client"], "    ")
        _actions(lines, "server", fl["server"], "    ")
    return "\n".join(lines) + "\n"


# -- generation -----------------------------------------------------------------------


def _delimiter(data: bytes, a: int, b: int) -> bytes | None:
    """A delimiter that makes ``expect pattern`` stop at ``b``: the interval ends with it
    and it occurs nowhere earlier in the interval. Only for text."""
    chunk = data[a:b]
    if not chunk or sum(1 for c in chunk if 32 <= c < 127 or c in (9, 10, 13)) < 0.9 * len(chunk):
        return None
    for d in _DELIMITERS:
        if chunk.endswith(d) and chunk.find(d) + len(d) == len(chunk):
            return d
    return None


def _expects(
    unit: Unit,
    analysis: Analysis,
    captures: list[tuple[int, int, str]],
    timeout_ms: int | None,
) -> list[dict[str, Any]]:
    """The ``expect`` actions that consume one unit exactly, with a ``set`` after each
    capture."""
    n = len(unit.data)
    extra: dict[str, Any] = {} if timeout_ms is None else {"timeout_ms": timeout_ms}
    if unit.udp:
        return [{"expect": {"bytes": n, **extra}}]
    cuts = {0, n}
    for r in analysis.records:
        cuts |= {r.start, r.end}
    for s, e, _ in captures:
        cuts |= {s, e}
    points = sorted(cuts)
    captured = {(s, e): v for s, e, v in captures}
    body_var = {(r.start, r.end): r.length_var for r in analysis.records if r.length_var}
    kinds = [(r.start, r.end, r.kind) for r in analysis.records]
    out: list[dict[str, Any]] = []
    for a, b in pairwise(points):
        kind = next((k for s, e, k in kinds if s <= a and b <= e), "raw")
        delim = (
            None if (a, b) in captured or kind in ("body", "frame") else _delimiter(unit.data, a, b)
        )
        if (a, b) in body_var:
            out.append({"expect": {"bytes": f"{{{{ vars.{body_var[(a, b)]} }}}}", **extra}})
        elif delim is not None:
            out.append({"expect": {"pattern": delim.decode("ascii"), **extra}})
        else:
            out.append({"expect": {"bytes": b - a, **extra}})
        if (a, b) in captured:
            out.append({"set": {captured[(a, b)]: "{{ last }}"}})
    return out


def _compose(unit: Unit, spans: list[Span]) -> str:
    spans = sorted(spans, key=lambda s: s.start)
    out, pos = [], 0
    for s in spans:
        out.append(escape_literal(unit.data[pos : s.start]))
        out.append(s.tmpl)
        pos = s.end
    out.append(escape_literal(unit.data[pos:]))
    return "".join(out)


def _check_overlap(unit: Unit, spans: list[Span]) -> None:
    last = 0
    for s in sorted(spans, key=lambda s: s.start):
        if s.start < last or s.end > len(unit.data) or s.end <= s.start:
            raise ScenarioError("internal error: overlapping replacement spans")
        last = s.end


@dataclass
class _Plan:
    units: list[Unit]
    analyses: list[Analysis]
    spans: list[list[Span]]
    sets: dict[int, list[tuple[str, str]]]
    captures: dict[int, list[tuple[int, int, str]]]
    alloc: VarAlloc
    fields: list[Field]
    counts: Counts


def _plan(
    units: list[Unit], refs: list[float], cfg: Config, alloc: VarAlloc, detect: bool
) -> _Plan:
    """Framing, fields, and the echo plan of every unit of a scenario."""
    local = Config(**{**cfg.__dict__, "detect_fields": detect})
    analyses = [
        analyze_framing(u.data, _DIRS[u.direction], local, alloc, datagram=u.udp) for u in units
    ]
    plan = _Plan(units, analyses, [list(a.spans) for a in analyses], {}, {}, alloc, [], Counts())
    for a in analyses:
        plan.counts.lengths += sum(1 for s in a.spans if s.kind == "length")
        plan.counts.fills += sum(1 for s in a.spans if s.kind in ("body", "payload", "opaque"))
        plan.counts.frames += len(a.records) if a.protocol == "frames" else 0
    if not detect:
        return plan
    plan.fields = detect_fields(
        [(_DIRS[u.direction], u.data) for u in units], analyses, refs, local, alloc
    )
    for f in plan.fields:
        _apply(plan, f)
    return plan


def _apply(plan: _Plan, f: Field) -> None:
    """Turn one field into spans, and add the ``set`` and capture that its echo needs."""
    units, occ = plan.units, f.occurrences
    ref = "{{ vars." + f.name + " }}"
    if f.kind == "http_date":
        plan.alloc.declare(f.name, f.value)
        plan.counts.http_dates += len(occ)
        for o in occ:
            plan.spans[o.unit].append(Span(o.start, o.end, ref, f.kind))
        return
    counter = {"cookie": "cookies", "id": "identifiers"}.get(f.kind, "epochs")
    setattr(plan.counts, counter, getattr(plan.counts, counter) + len(occ))
    if len(occ) == 1:
        o = occ[0]
        plan.spans[o.unit].append(Span(o.start, o.end, f.gen, f.kind))
        return
    first = occ[0]
    origin = units[first.unit].direction
    others = {units[o.unit].direction for o in occ} - {origin}
    if others and units[first.unit].udp:
        # A datagram cannot be consumed in parts, so the other side cannot read the value.
        # Both sides read it from a scenario variable that keeps the captured value.
        plan.alloc.declare(f.name, f.value)
        plan.counts.static_vars += 1
    else:
        plan.sets.setdefault(first.unit, []).append((f.name, f.gen))
        if others:
            plan.captures.setdefault(first.unit, []).append((first.start, first.end, f.name))
            plan.counts.captures += 1
    for o in occ:
        plan.spans[o.unit].append(Span(o.start, o.end, ref, f.kind))


def _verify(plan: _Plan) -> str | None:
    """Render every template and every generator, and compare lengths with the capture.

    Returns a description of the first mismatch, or None."""
    values: dict[str, Any] = dict(plan.alloc.values)
    for f in plan.fields:
        values.setdefault(f.name, f.value)
    for f in plan.fields:
        if f.gen and len(render(f.gen, values)) != len(f.value.encode("latin-1")):
            return f"generator of {f.name} changes the length"
    for i, u in enumerate(plan.units):
        _check_overlap(u, plan.spans[i])
        got = len(render(_compose(u, plan.spans[i]), values))
        if got != len(u.data):
            return f"unit {i} renders {got} bytes, captured {len(u.data)}"
    return None


def _close_actions(session: dict[str, Any]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    if session["protocol"] == "udp":
        return [{"close": "fin"}], [{"close": "fin"}]
    if session["client_to_server"].get("rst"):
        return [{"close": "rst"}], []
    if session["server_to_client"].get("rst"):
        return [], [{"close": "rst"}]
    return [{"close": "fin"}], [{"close": "fin"}]


def build_scenario(
    name: str,
    sessions: list[dict[str, Any]],
    module: str,
    cfg: Config,
    *,
    combined: bool,
) -> Scenario | None:
    """Build one scenario from one session, or from several that run in order.

    Returns None when no session carries payload."""
    warnings: list[str] = []
    alloc = VarAlloc()
    flows_info: list[FlowInfo] = []
    units: list[Unit] = []
    refs: list[float] = []
    ports: dict[int, str] = {}
    for session in sessions:
        if not session["turns"]:
            warnings.append(f"session {session['id']} carries no payload and was skipped")
            continue
        if not session["roles"]["client_known"]:
            warnings.append(
                f"session {session['id']} has no handshake; the roles come from the ports"
            )
        if session["anomalies"]:
            warnings.append(
                f"session {session['id']} has anomalies: {', '.join(session['anomalies'])}"
            )
        port = session["server"]["port"]
        if port not in ports:
            ports[port] = alloc.unique("server_port")
            alloc.declare(ports[port], port)
        flows_info.append(FlowInfo(session, ports[port]))
        flow_units = _units(session, len(flows_info) - 1, warnings)
        units += flow_units
        refs += [session["start_ns"] / 1e9] * len(flow_units)
    if not units:
        return None

    plan = _plan(units, refs, cfg, alloc, cfg.detect_fields)
    problem = _verify(plan)
    if problem is not None:
        warnings.append(f"field detection was switched off for {name}: {problem}")
        alloc_literal = VarAlloc()
        for k, v in alloc.values.items():
            if k.startswith("server_port"):
                alloc_literal.declare(k, v)
        plan = _plan(units, refs, cfg, alloc_literal, False)
        if _verify(plan) is not None:
            raise ScenarioError(f"internal error: literal scenario {name} does not render")

    flows: list[dict[str, Any]] = []
    for idx, info in enumerate(flows_info):
        session = info.session
        udp = session["protocol"] == "udp"
        connect = {"connect": {"port": f"{{{{ vars.{info.port_var} }}}}"}}
        accept = {"accept": {"port": f"{{{{ vars.{info.port_var} }}}}"}}
        client: list[dict[str, Any]] = [connect]
        server: list[dict[str, Any]] = [accept]
        prev_end: int | None = None
        for i, u in enumerate(plan.units):
            if u.flow != idx:
                continue
            gap_ms = 0 if prev_end is None else max(0, (u.start_ns - prev_end) // 1_000_000)
            prev_end = u.end_ns
            send_side, recv_side = (client, server) if u.direction == _UP else (server, client)
            timeout = None
            if cfg.keep_timing and gap_ms >= cfg.min_delay_ms and u.first_in_turn:
                send_side.append({"delay": {"ms": int(gap_ms)}})
                timeout = int(gap_ms) + 10_000
            if plan.sets.get(i):
                send_side.append({"set": {k: v for k, v in plan.sets[i]}})
            send_side.append({"send": _compose(u, plan.spans[i])})
            recv_side += _expects(u, plan.analyses[i], plan.captures.get(i, []), timeout)
        c_close, s_close = _close_actions(session)
        client += c_close
        server += s_close
        fl: dict[str, Any] = {
            "id": f"s{session['id']}" if combined else "main",
            "module": module,
            "client": client,
            "server": server,
        }
        if udp:
            fl["l4"] = "udp"
        if idx > 0:
            fl["after"] = flows[idx - 1]["id"]
        flows.append(fl)

    ids = [i.session["id"] for i in flows_info]
    kinds = "UDP" if all(i.session["protocol"] == "udp" for i in flows_info) else "TCP"
    description = (
        f"{kinds} session{'s' if len(ids) > 1 else ''} {', '.join(map(str, ids))} "
        "of a capture, generated by python3 -m capture scenario."
    )
    text = scenario_yaml(name, description, plan.alloc.values, flows)
    return Scenario(name, text, sum(len(u.data) for u in plan.units), plan.counts, warnings, ids)


# -- module ---------------------------------------------------------------------------


def manifest_yaml(
    module: str,
    scenarios: list[Scenario],
    *,
    license_id: str,
    provenance: dict[str, Any],
) -> str:
    """Write ``manifest.yaml`` for the generated module."""
    n = sum(len(s.sessions) for s in scenarios)
    plural = "" if n == 1 else "s"
    lines = [
        "apiVersion: fixingpoint.dev/v1alpha1",
        "kind: Module",
        f"name: {module}",
        "version: 0.1.0",
        "type: app",
        "summary: " + json.dumps(f"Scenarios generated from {n} captured session{plural}."),
        f"engine_api: {json.dumps(ENGINE_API)}",
        f"license: {license_id}",
        "scenarios:",
        *[f"  - scenarios/{s.name}.yaml" for s in scenarios],
        "expected_bytes:",
        *[f"  {s.name}: {s.expected_bytes}" for s in scenarios],
        "provenance:",
        *[f"  {k}: {_flow(v)}" for k, v in provenance.items()],
    ]
    return "\n".join(lines) + "\n"


def provenance_record(
    *, synthetic: bool, consent: str | None, scrubbed: bool, source_name: str
) -> tuple[dict[str, Any], list[str]]:
    """The provenance block of the manifest, and the warnings about it.

    A module from a capture validates only with a consent note and ``scrubbed: true``,
    which CAP-03 establishes. Without them the manifest says what is true, and the
    validator rejects it, so unscrubbed content cannot enter a module tree unnoticed."""
    notes = f"Generated by python3 -m capture scenario from the sessions of {source_name}."
    if synthetic:
        return {"source": "synthetic", "notes": notes}, []
    record: dict[str, Any] = {
        "source": "capture",
        "consent": consent or "not recorded",
        "scrubbed": scrubbed,
        "notes": notes,
    }
    warnings = []
    if not scrubbed or not consent:
        warnings.append(
            "the manifest records no consent or no scrubbing, so tools/contracts/validate.py "
            "rejects it; scrub the sessions (CAP-03) and pass --consent TEXT --scrubbed, or "
            "pass --synthetic for a capture that you generated yourself"
        )
    return record, warnings


def generate(
    doc: dict[str, Any],
    sessions: list[dict[str, Any]],
    module: str,
    cfg: Config,
    *,
    combine: str | None,
    license_id: str,
    provenance: dict[str, Any],
) -> tuple[dict[str, str], dict[str, Any]]:
    """Return the files of the module (path to text) and a report."""
    if not IDENT.match(module):
        raise ScenarioError(
            f"the module name {module!r} is not an identifier of lower case letters, digits, "
            "and underscores; name the output directory accordingly"
        )
    if combine is not None and not IDENT.match(combine):
        raise ScenarioError(f"the scenario name {combine!r} is not a valid identifier")
    groups = [(combine, sessions)] if combine else [(f"session_{s['id']}", [s]) for s in sessions]
    scenarios = [
        sc
        for name, group in groups
        if (sc := build_scenario(name, group, module, cfg, combined=combine is not None))
    ]
    if not scenarios:
        raise ScenarioError("none of the selected sessions carries payload bytes")
    files = {f"scenarios/{s.name}.yaml": s.text for s in scenarios}
    files["manifest.yaml"] = manifest_yaml(
        module, scenarios, license_id=license_id, provenance=provenance
    )
    report = {
        "module": module,
        "source": doc["source"]["file"],
        "scenarios": [
            {
                "scenario": s.name,
                "sessions": s.sessions,
                "expected_bytes": s.expected_bytes,
                "detected": s.counts.as_dict(),
                "warnings": s.warnings,
            }
            for s in scenarios
        ],
    }
    return files, report
