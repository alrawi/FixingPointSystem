# SPDX-License-Identifier: GPL-3.0-only
"""A small model of the flow interpreter, for tests of generated scenarios.

The model runs the client program and the server program of one flow against each other
over in-memory byte streams (TCP) or datagram queues (UDP), with the stop rules of
``contracts/README.md``. It evaluates templates with ``capture.templates.render`` and
binds ``set`` results the way the engine does (``Value::parse_override``: integers, then
floats, then text). It does not replace the engine. The replay test that runs the real
agent in the namespace harness does that. It catches deadlocks, wrong stop rules, and
byte counts that differ from the capture without a network.
"""

from __future__ import annotations

import random
import re
from dataclasses import dataclass, field
from decimal import Decimal
from typing import Any

from capture.templates import render

_INT = re.compile(r"[+-]?[0-9]+")
_FLOAT = re.compile(
    r"[+-]?(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][+-]?[0-9]+)?|[+-]?(?:inf|infinity|nan)"
)


def parse_override(text: str) -> int | float | bool | str:
    """The engine's rule for ``set`` results and command line overrides."""
    if _INT.fullmatch(text) and -(2**63) <= int(text) < 2**63:
        return int(text)
    if _FLOAT.fullmatch(text.lower()) or _FLOAT.fullmatch(text):
        return float(text)
    if text in ("true", "false"):
        return text == "true"
    return text


def display(value: Any) -> str:
    """How a variable renders inside a template."""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, float):
        if value != value or value in (float("inf"), float("-inf")):
            return str(value)
        text = format(Decimal(repr(value)), "f")
        return text[:-2] if text.endswith(".0") else text
    return str(value)


class Deadlock(AssertionError):
    """Both sides wait and neither can continue."""


@dataclass
class _Side:
    name: str
    actions: list[dict[str, Any]]
    vars: dict[str, Any]
    udp: bool
    pos: int = 0
    last: bytes = b""
    inbox: bytearray = field(default_factory=bytearray)
    datagrams: list[bytes] = field(default_factory=list)
    sent: int = 0
    spawned: list[str] = field(default_factory=list)
    received: int = 0
    done: bool = False


class _Vars(dict[str, Any]):
    """Variables as strings for the renderer, so ``{{ vars.x }}`` prints like the engine."""

    def __getitem__(self, key: str) -> Any:
        return display(super().__getitem__(key))


def run_flow(
    flow: dict[str, Any],
    scenario_vars: dict[str, Any],
    seed: int = 0,
    inherited: dict[str, dict[str, Any]] | None = None,
) -> dict[str, dict[str, Any]]:
    """Run one flow and return the final variables, byte counts, and spawns of each side.

    ``inherited`` holds the variables that a parent flow handed to each side."""
    rng = random.Random(seed)
    udp = flow.get("l4") == "udp"
    sides = {
        n: _Side(n, flow[n], _Vars({**scenario_vars, **(inherited or {}).get(n, {})}), udp)
        for n in ("client", "server")
    }
    peer = {"client": sides["server"], "server": sides["client"]}
    for _ in range(100_000):
        progressed = False
        for side in sides.values():
            while not side.done and _step(side, peer[side.name], rng):
                progressed = True
        if all(s.done for s in sides.values()):
            return {
                n: {
                    "vars": dict(s.vars),
                    "sent": s.sent,
                    "received": s.received,
                    "spawned": s.spawned,
                }
                for n, s in sides.items()
            }
        if not progressed:
            where = {n: (s.pos, s.actions[s.pos]) for n, s in sides.items() if not s.done}
            raise Deadlock(f"deadlock at {where}")
    raise Deadlock("no end")


def run_scenario(doc: dict[str, Any], seed: int = 0) -> list[dict[str, dict[str, Any]]]:
    """Run the first flow of a scenario and every flow that it spawns, in order.

    A spawned flow starts with the variables that its parent side holds, as the engine
    does. Returns the result of each flow."""
    flows = {f["id"]: f for f in doc["flows"]}
    queue: list[tuple[str, dict[str, dict[str, Any]] | None]] = [
        (next(f["id"] for f in doc["flows"] if not f.get("spawned")), None)
    ]
    out = []
    while queue:
        flow_id, inherited = queue.pop(0)
        result = run_flow(flows[flow_id], doc.get("vars", {}), seed, inherited)
        out.append(result)
        for child in result["client"]["spawned"]:
            queue.append((child, {n: r["vars"] for n, r in result.items()}))
    return out


def _count(side: _Side, value: Any, rng: random.Random) -> int:
    if isinstance(value, int):
        return value
    return int(render(value, side.vars, last=side.last, rng=rng))


def _step(side: _Side, other: _Side, rng: random.Random) -> bool:
    """Run the next action of ``side``. Return False when it must wait."""
    if side.pos >= len(side.actions):
        side.done = True
        return True
    ((kind, body),) = side.actions[side.pos].items()
    if kind in ("connect", "accept", "close", "delay"):
        pass
    elif kind == "set":
        for k, tmpl in body.items():
            side.vars[k] = parse_override(render(tmpl, side.vars, last=side.last, rng=rng).decode())
    elif kind == "spawn":
        side.spawned.append(body["flow"])
    elif kind == "send":
        data = render(body, side.vars, last=side.last, rng=rng)
        side.sent += len(data)
        if side.udp:
            other.datagrams.append(data)
        else:
            other.inbox += data
    elif kind == "expect":
        if not _expect(side, body, rng):
            return False
    else:
        raise AssertionError(f"the model does not run {kind}")
    side.pos += 1
    return True


def _expect(side: _Side, rule: dict[str, Any], rng: random.Random) -> bool:
    if side.udp:
        if not side.datagrams:
            return False
        data = side.datagrams.pop(0)
        if "bytes" in rule:
            assert len(data) == _count(side, rule["bytes"], rng), "datagram size"
        side.last = data
        side.received += len(data)
        return True
    buf = side.inbox
    if "pattern" in rule:
        at = bytes(buf).find(rule["pattern"].encode())
        if at < 0:
            return False
        take = at + len(rule["pattern"])
    elif "bytes" in rule:
        take = _count(side, rule["bytes"], rng)
        if len(buf) < take:
            return False
    else:
        take = _count(side, rule["min_bytes"], rng)
        if len(buf) < take:
            return False
        take = len(buf)
    side.last = bytes(buf[:take])
    del buf[:take]
    side.received += take
    return True
