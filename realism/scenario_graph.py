# SPDX-License-Identifier: GPL-3.0-only
"""Read the flow graph of a scenario file for the session structure check.

A scenario of ``contracts/schemas/scenario.schema.json`` groups flows. Each flow runs
one session, so a flow is one connection of the capture (one TCP session or one UDP
exchange). The graph keeps what the capture can show:

* the flow ids, their ``l4`` transport, and their server port when the file fixes it
  (a literal or ``{{ vars.name }}`` with a numeric variable),
* the ``after`` edges, which say that every connection of the later flow starts once
  every connection of the earlier flow completed,
* the ``spawn`` edges, with how many connections one parent connection spawns. A
  ``spawn`` inside a ``loop`` repeats ``count`` times, and a ``spawn`` inside a
  ``branch`` may not happen.

The reader uses no template engine. A count or a port that is not a literal or a plain
``vars`` reference is unknown, and the check then accepts any number or any port.
"""

from __future__ import annotations

import importlib
import re
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

_VAR_REFERENCE = re.compile(r"^\s*\{\{\s*vars\.([a-z][a-z0-9_]*)\s*\}\}\s*$")


class ScenarioError(Exception):
    """A scenario file cannot be read or does not describe a flow graph."""


@dataclass(frozen=True)
class Multiplicity:
    """How many connections of a flow one scenario instance holds.

    ``high`` is None when the number depends on a value that the file does not fix.
    """

    low: int
    high: int | None

    def __add__(self, other: Multiplicity) -> Multiplicity:
        high = None if self.high is None or other.high is None else self.high + other.high
        return Multiplicity(self.low + other.low, high)

    def times(self, count: Multiplicity) -> Multiplicity:
        """Return the multiplicity of this body repeated ``count`` times."""
        if count.high is None:
            high = None if self.high != 0 else 0
        elif self.high is None:
            high = None if count.high != 0 else 0
        else:
            high = self.high * count.high
        return Multiplicity(self.low * count.low, high)

    def allows(self, observed: int) -> bool:
        """Return True when ``observed`` connections fit."""
        return observed >= self.low and (self.high is None or observed <= self.high)

    def text(self) -> str:
        """Return the multiplicity as words for a failure message."""
        if self.high is None:
            return f"at least {self.low}"
        if self.low == self.high:
            return str(self.low)
        return f"{self.low} to {self.high}"


ZERO = Multiplicity(0, 0)
ONE = Multiplicity(1, 1)


@dataclass
class FlowNode:
    """One flow of the graph with the facts that identify its connection."""

    id: str
    module: str
    transport: str
    after: tuple[str, ...]
    spawned: bool
    server_port: int | None
    spawn_order: int = 0
    multiplicity: Multiplicity = ONE


@dataclass(frozen=True)
class Edge:
    """An ordering constraint between two flows, ``kind`` being ``after`` or ``spawn``."""

    kind: str
    source: str
    target: str

    def text(self) -> str:
        """Return the edge the way a failure names it."""
        return f"{self.kind} {self.source} -> {self.target}"


@dataclass
class ScenarioGraph:
    """The flows and edges of one scenario file."""

    name: str
    source: str
    flows: list[FlowNode]
    edges: list[Edge] = field(default_factory=list)

    def flow(self, flow_id: str) -> FlowNode:
        """Return the flow with this id."""
        for node in self.flows:
            if node.id == flow_id:
                return node
        raise KeyError(flow_id)

    def fixed_ports(self, transport: str) -> set[int]:
        """Return the server ports that the file fixes for flows of a transport."""
        return {
            f.server_port
            for f in self.flows
            if f.transport == transport and f.server_port is not None
        }


def load(path: Path, overrides: Mapping[str, Any] | None = None) -> ScenarioGraph:
    """Read a scenario file and build its graph.

    ``overrides`` replaces scenario variables, as a test parameter does, so a port that
    a run changes still resolves.
    """
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise ScenarioError(f"read scenario {path}: {exc}") from exc
    try:
        yaml: Any = importlib.import_module("yaml")
    except ImportError as exc:
        raise ScenarioError(f"parse scenario {path}: YAML needs PyYAML") from exc
    try:
        doc = yaml.safe_load(text)
    except yaml.YAMLError as exc:
        raise ScenarioError(f"parse scenario {path}: {exc}") from exc
    return parse(doc, source=str(path), overrides=overrides)


def parse(
    doc: Any, source: str = "<scenario>", overrides: Mapping[str, Any] | None = None
) -> ScenarioGraph:
    """Build the graph of a decoded scenario document."""
    where = f"scenario {source}"
    if not isinstance(doc, dict) or not isinstance(doc.get("scenario"), str):
        raise ScenarioError(f"{where}: the document must be a mapping with a scenario name")
    raw_flows = doc.get("flows")
    if not isinstance(raw_flows, list) or not raw_flows:
        raise ScenarioError(f"{where}: flows must be a non empty list")
    variables: dict[str, Any] = dict(doc.get("vars") or {})
    variables.update(overrides or {})
    flows: list[FlowNode] = []
    spawns: dict[str, dict[str, list[Multiplicity]]] = {}
    for index, raw in enumerate(raw_flows):
        here = f"{where} flows[{index}]"
        if not isinstance(raw, dict) or not isinstance(raw.get("id"), str):
            raise ScenarioError(f"{here}: a flow is a mapping with an id")
        flow_id = raw["id"]
        if any(f.id == flow_id for f in flows):
            raise ScenarioError(f"{here}: duplicate flow id {flow_id!r}")
        l4 = raw.get("l4", "tcp")
        if l4 not in ("tcp", "udp"):
            raise ScenarioError(f"{here}: l4 must be tcp or udp")
        after = raw.get("after", [])
        after_ids = [after] if isinstance(after, str) else list(after)
        if not all(isinstance(a, str) for a in after_ids):
            raise ScenarioError(f"{here}: after must name flow ids")
        client = raw.get("client") or []
        server = raw.get("server") or []
        port = _port(server, "accept", variables)
        if port is None:
            port = _port(client, "connect", variables)
        flows.append(
            FlowNode(
                id=flow_id,
                module=str(raw.get("module", "")),
                transport=l4,
                after=tuple(after_ids),
                spawned=bool(raw.get("spawned", False)),
                server_port=port,
            )
        )
        by_side = [_spawns(client, variables), _spawns(server, variables)]
        spawns[flow_id] = {}
        for side in by_side:
            for target, _ in side:
                if target not in spawns[flow_id]:
                    spawns[flow_id][target] = []
        for target, sides in spawns[flow_id].items():
            # Both sides of the parent start the same child, so each side that spawns it
            # gives its own total, and the later steps intersect them.
            for side in by_side:
                mine = [m for t, m in side if t == target]
                if mine:
                    sides.append(_total(mine))
    ids = {f.id for f in flows}
    edges: list[Edge] = []
    for node in flows:
        for earlier in node.after:
            if earlier not in ids:
                raise ScenarioError(f"{where}: flow {node.id} runs after unknown flow {earlier}")
            edges.append(Edge("after", earlier, node.id))
    order = 0
    for node in flows:
        for target in spawns[node.id]:
            if target not in ids:
                raise ScenarioError(f"{where}: flow {node.id} spawns unknown flow {target}")
            edges.append(Edge("spawn", node.id, target))
    for node in flows:
        for target in spawns[node.id]:
            child = next(f for f in flows if f.id == target)
            if child.spawn_order == 0:
                order += 1
                child.spawn_order = order
    graph = ScenarioGraph(doc["scenario"], source, flows, edges)
    for node in flows:
        node.multiplicity = _multiplicity(node.id, graph, spawns, ())
    return graph


def _port(actions: Any, kind: str, variables: Mapping[str, Any]) -> int | None:
    """Return the port of the first ``connect`` or ``accept`` action that resolves."""
    for action in _walk(actions):
        spec = action.get(kind)
        if isinstance(spec, dict):
            value = _resolve(spec.get("port"), variables)
            if value is not None:
                return value
            return None
    return None


def _walk(actions: Any) -> list[dict[str, Any]]:
    """Yield every action in document order, descending into loops and branches."""
    out: list[dict[str, Any]] = []
    if not isinstance(actions, list):
        return out
    for action in actions:
        if not isinstance(action, dict):
            continue
        out.append(action)
        loop = action.get("loop")
        if isinstance(loop, dict):
            out += _walk(loop.get("do"))
        branch = action.get("branch")
        if isinstance(branch, list):
            for arm in branch:
                if isinstance(arm, dict):
                    out += _walk(arm.get("do"))
    return out


def _resolve(value: Any, variables: Mapping[str, Any]) -> int | None:
    """Return an integer that a literal or a plain ``vars`` reference gives, else None."""
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        match = _VAR_REFERENCE.match(value)
        if match:
            return _resolve(variables.get(match.group(1)), variables)
        if value.strip().isdigit():
            return int(value.strip())
    return None


def _spawns(actions: Any, variables: Mapping[str, Any]) -> list[tuple[str, Multiplicity]]:
    """Return the spawned flow ids of a program, each with its number of spawns.

    A target spawned at two places of one program is listed twice, and the caller adds
    the counts.
    """
    found: list[tuple[str, Multiplicity]] = []
    if not isinstance(actions, list):
        return found
    for action in actions:
        if not isinstance(action, dict):
            continue
        spawn = action.get("spawn")
        if isinstance(spawn, dict) and isinstance(spawn.get("flow"), str):
            found.append((spawn["flow"], ONE))
        loop = action.get("loop")
        if isinstance(loop, dict):
            count = _resolve(loop.get("count"), variables)
            repeat = Multiplicity(count, count) if count is not None else Multiplicity(0, None)
            found += [(t, m.times(repeat)) for t, m in _spawns(loop.get("do"), variables)]
        branch = action.get("branch")
        if isinstance(branch, list):
            found += _branch_spawns(branch, variables)
    return found


def _branch_spawns(
    branch: list[Any], variables: Mapping[str, Any]
) -> list[tuple[str, Multiplicity]]:
    arms = [_spawns(a.get("do"), variables) for a in branch if isinstance(a, dict)]
    targets = list(dict.fromkeys(t for arm in arms for t, _ in arm))
    out = []
    for target in targets:
        per_arm = [_total([m for t, m in arm if t == target]) for arm in arms]
        low = min(m.low for m in per_arm)
        highs = [m.high for m in per_arm]
        high = None if any(h is None for h in highs) else max(h for h in highs if h is not None)
        out.append((target, Multiplicity(low, high)))
    return out


def _total(items: list[Multiplicity]) -> Multiplicity:
    total = ZERO
    for item in items:
        total = total + item
    return total


def _multiplicity(
    flow_id: str,
    graph: ScenarioGraph,
    spawns: Mapping[str, Mapping[str, list[Multiplicity]]],
    seen: tuple[str, ...],
) -> Multiplicity:
    node = graph.flow(flow_id)
    if not node.spawned:
        return ONE
    if flow_id in seen:
        return Multiplicity(0, None)
    total = ZERO
    for parent_id, targets in spawns.items():
        counts = targets.get(flow_id)
        if counts is None:
            continue
        # The two sides of the parent each count their spawns of this flow. Both start
        # the same connections, so the bounds of the sides all hold.
        low = max(m.low for m in counts)
        highs = [m.high for m in counts if m.high is not None]
        per_parent = Multiplicity(low, min(highs) if highs else None)
        parent = _multiplicity(parent_id, graph, spawns, (*seen, flow_id))
        total = total + per_parent.times(parent)
    if total == ZERO and not any(flow_id in t for t in spawns.values()):
        # A spawned flow that nothing spawns never runs.
        return ZERO
    return total
