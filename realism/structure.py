# SPDX-License-Identifier: GPL-3.0-only
"""Session structure check of the realism runner.

A scenario instance opens several connections, for example a DNS lookup and then a page
fetch, or an FTP control connection and one data connection per transfer. A generator
that drops a connection, repeats one, or runs them in the wrong order produces traffic
that no real client makes. The check reads the flow graph of the scenario file, rebuilds
the connections of each instance from the capture, and compares both.

**Graph.** :mod:`realism.scenario_graph` reads the flows, the ``after`` edges, and the
``spawn`` edges of a scenario file. Each flow is one connection. A flow that is not
spawned occurs once per instance, and a spawned flow occurs as often as its parent
spawns it, so a ``spawn`` in a ``loop`` of ``count`` repeats and a ``spawn`` in a
``branch`` may not happen.

**Which connection is which flow.** A session belongs to a flow when the transport
agrees (``l4``) and the server port agrees. The port comes from the ``accept`` of the
server program, or from the ``connect`` of the client program, as a literal or a
``{{ vars.name }}`` reference. A flow without such a port, for example an FTP data
connection whose port a reply announces, matches every session of its transport on a
port that no other flow of the scenario fixes. Flows that the capture cannot tell apart
are matched in the order in which the scenario spawns them.

**Which connections form an instance.** The runner groups the sessions of the scenario
by client address and walks them in order of their first packet. A session joins the
open instance of its client when that instance still has room for its flow, meaning that
the number of connections of the flow is below the highest number that the graph allows.
Otherwise the session opens a new instance. With ``structure.max_gap_s`` a client that
was silent for longer than the gap also opens a new instance. A session that matches no
flow of any scenario, or that lies outside the ``match`` of every scenario entry, is
counted in ``sessions_unassigned`` and not judged.

The rule fills the slots of the graph and does not read the order of connections, so a
page fetch that comes before its lookup still joins the instance of the lookup, and the
ordering check then names the broken edge. Its limits are

* a client address that runs several instances at the same time cannot be split, and the
  extra connections open an instance of their own that lacks its other flows,
* clients behind one address look like one client,
* a connection that is missing or repeated shifts the later connections of that client
  into other instances, so one fault can fail the next instance too, and the failure of
  an instance that opened because the previous one was full says so, and
* flows with the same transport and without a fixed port are interchangeable, so the
  check counts them but cannot tell which of them a connection is.

**Comparison.** Per instance and flow the number of connections must lie within the
multiplicity of the graph. Per ``after`` edge every connection of the later flow must
start no earlier than every connection of the earlier flow completed, where a session
completes at its last packet with payload, FIN, or RST (the closing ACK does not count,
because the engine starts the next flow when the close action returns). Per ``spawn``
edge every connection of the child must start while some connection of the parent runs,
between its first and its last packet. The slack ``structure.tolerance_s`` (default 0)
loosens both comparisons.
"""

from __future__ import annotations

from collections import defaultdict
from collections.abc import Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from realism import scenario_graph
from realism.capture import Session, SessionKey, decode, read_frames
from realism.labels import Labels, Rule, StructureLabels, StructureScenario
from realism.scenario_graph import Edge, FlowNode, ScenarioGraph
from realism.series import failures_series

CHECK_NAME = "session_structure"
METRIC = "realism.structure_failures"
MAX_FAILURES_LISTED = 50
MAX_INSTANCES_LISTED = 50
_TCP_FIN = 0x01
_TCP_RST = 0x04


@dataclass(frozen=True)
class Connection:
    """One session of a capture with the times that the ordering rules read."""

    session: Session
    start: float
    end: float

    @property
    def last(self) -> float:
        """The time of the last packet of any kind."""
        return self.session.last_seen

    def describe(self) -> str:
        """Return the session the way a failure names it."""
        s = self.session
        return (
            f"session {s.id}, {s.transport} {s.client_ip}:{s.client_port} -> "
            f"{s.server_ip}:{s.server_port}"
        )


@dataclass
class Instance:
    """The connections of one scenario instance, grouped by flow."""

    number: int
    graph: ScenarioGraph
    client_ip: str
    opened_by: str | None = None
    members: list[tuple[str, Connection]] = field(default_factory=list)
    failures: list[str] = field(default_factory=list)

    @property
    def start(self) -> float:
        """The first packet of the first connection."""
        return min(c.start for _, c in self.members)

    @property
    def last_activity(self) -> float:
        """The last packet of any connection so far."""
        return max(c.last for _, c in self.members)

    def count(self, flow_id: str) -> int:
        """Return the number of connections that the instance holds for a flow."""
        return sum(1 for f, _ in self.members if f == flow_id)

    def connections(self, flow_id: str) -> list[Connection]:
        """Return the connections of a flow in start order."""
        return sorted((c for f, c in self.members if f == flow_id), key=lambda c: c.start)

    @property
    def label(self) -> str:
        """Return the instance the way a failure names it."""
        return (
            f"scenario {self.graph.name} instance {self.number} "
            f"(client {self.client_ip}, first packet {self.start:.6f})"
        )

    def at(self, moment: float) -> str:
        """Return a time relative to the start of the instance."""
        return f"{moment - self.start:+.6f} s"


@dataclass
class ScenarioResult:
    """The outcome of one scenario entry."""

    graph: ScenarioGraph
    spec: StructureScenario | None
    instances: list[Instance]
    sessions: int
    failures: list[str]


@dataclass
class StructureResult:
    """The outcome of the session structure check."""

    max_gap_s: float | None
    tolerance_s: float
    sessions_total: int
    sessions_unassigned: int
    scenarios: list[ScenarioResult]
    failures: list[str]

    @property
    def passed(self) -> bool:
        """True when nothing in ``failures`` stands against the capture."""
        return not self.failures

    def metrics(self) -> list[dict[str, Any]]:
        """Return the broken rules of each scenario, and the failures series.

        Two scenarios of one name are one series with the summed value, so the values add up to
        ``failures_total``.
        """
        broken: dict[str, int] = {}
        for scenario in self.scenarios:
            name = scenario.graph.name
            broken[name] = broken.get(name, 0) + len(scenario.failures)
        return [
            *(
                {"name": METRIC, "labels": {"scenario": name}, "value": count}
                for name, count in broken.items()
            ),
            failures_series(CHECK_NAME, len(self.failures), self.passed),
        ]

    def to_dict(self) -> dict[str, Any]:
        """Return the check entry of the realism report."""
        instances_total = sum(len(s.instances) for s in self.scenarios)
        failed = [i for s in self.scenarios for i in s.instances if i.failures]
        return {
            "check": CHECK_NAME,
            "metrics": self.metrics(),
            "passed": self.passed,
            "max_gap_s": self.max_gap_s,
            "tolerance_s": self.tolerance_s,
            "sessions_total": self.sessions_total,
            "sessions_assigned": self.sessions_total - self.sessions_unassigned,
            "sessions_unassigned": self.sessions_unassigned,
            "instances": instances_total,
            "failed_instances": len(failed),
            "scenarios": [_scenario_entry(s) for s in self.scenarios],
            "failed_instance_details": [_instance_entry(i) for i in failed[:MAX_INSTANCES_LISTED]],
            "failures_total": len(self.failures),
            "failures": self.failures[:MAX_FAILURES_LISTED],
        }


def _scenario_entry(result: ScenarioResult) -> dict[str, Any]:
    graph = result.graph
    failed = sum(1 for i in result.instances if i.failures)
    return {
        "scenario": graph.name,
        "source": graph.source,
        "expected_instances": result.spec.expected_instances if result.spec else None,
        "instances": len(result.instances),
        "failed_instances": failed,
        "sessions": result.sessions,
        "flows": [
            {
                "id": f.id,
                "module": f.module,
                "transport": f.transport,
                "server_port": f.server_port,
                "spawned": f.spawned,
                "after": list(f.after),
                "min_connections": f.multiplicity.low,
                "max_connections": f.multiplicity.high,
                "connections": sum(i.count(f.id) for i in result.instances),
            }
            for f in graph.flows
        ],
        "edges": [{"kind": e.kind, "from": e.source, "to": e.target} for e in graph.edges],
    }


def _instance_entry(instance: Instance) -> dict[str, Any]:
    return {
        "scenario": instance.graph.name,
        "instance": instance.number,
        "client_ip": instance.client_ip,
        "start": round(instance.start, 6),
        "opened_by": instance.opened_by,
        "connections": [
            {
                "flow": flow,
                "session": c.session.id,
                "start": round(c.start - instance.start, 6),
                "end": round(c.end - instance.start, 6),
            }
            for flow, c in sorted(instance.members, key=lambda m: m[1].start)
        ],
        "failures": instance.failures,
    }


def connections(capture: Path, sessions: Sequence[Session]) -> dict[int, Connection]:
    """Return the start and the completion time of every session, keyed by session id.

    A session completes at its last packet with payload, FIN, or RST. A session without
    any of them completes at its last packet.
    """
    by_key: dict[SessionKey, Session] = {s.key: s for s in sessions}
    ends: dict[int, float] = {}
    _, frames = read_frames(capture)
    for frame in frames:
        packet = decode(frame)
        if packet is None:
            continue
        session = by_key.get(
            SessionKey.of(
                packet.transport, packet.src_ip, packet.src_port, packet.dst_ip, packet.dst_port
            )
        )
        if session is None:
            continue
        if packet.payload_len > 0 or packet.tcp_flags & (_TCP_FIN | _TCP_RST):
            ends[session.id] = max(ends.get(session.id, 0.0), packet.timestamp)
    return {s.id: Connection(s, s.first_seen, ends.get(s.id, s.last_seen)) for s in sessions}


def _candidates(graph: ScenarioGraph, session: Session) -> list[FlowNode]:
    """Return the flows that a session may be, the flows with a fixed port first."""
    fixed = graph.fixed_ports(session.transport)
    exact = [
        f
        for f in graph.flows
        if f.transport == session.transport and f.server_port == session.server_port
    ]
    free = [
        f
        for f in graph.flows
        if f.transport == session.transport
        and f.server_port is None
        and session.server_port not in fixed
    ]
    free.sort(key=lambda f: (f.spawn_order == 0, f.spawn_order))
    return exact + free


def _has_room(instance: Instance, flow: FlowNode) -> bool:
    high = flow.multiplicity.high
    return high is None or instance.count(flow.id) < high


def _full_reason(instance: Instance, candidates: list[FlowNode]) -> str:
    held = ", ".join(
        f"{f.id} already has {instance.count(f.id)} of {f.multiplicity.text()}"
        for f in candidates
        if f.multiplicity.high is not None
    )
    return f"instance {instance.number} has no room for it ({held})"


def build_instances(
    graph: ScenarioGraph,
    sessions: Sequence[Connection],
    max_gap_s: float | None,
) -> list[Instance]:
    """Group the sessions of one scenario into instances, per client in start order."""
    by_client: dict[str, list[Connection]] = defaultdict(list)
    for conn in sessions:
        by_client[conn.session.client_ip].append(conn)
    instances: list[Instance] = []
    for client in sorted(by_client):
        current: Instance | None = None
        for conn in sorted(by_client[client], key=lambda c: (c.start, c.session.id)):
            candidates = _candidates(graph, conn.session)
            room = [f for f in candidates if _has_room(current, f)] if current else []
            reason: str | None = None
            flow: FlowNode | None = room[0] if room else None
            if current is not None and flow is None:
                if all(f.multiplicity.high == 0 for f in candidates):
                    # The graph allows no connection like this one, and the count check
                    # reports it on the open instance.
                    flow = candidates[0]
                else:
                    reason = _full_reason(current, candidates)
            if (
                current is not None
                and flow is not None
                and max_gap_s is not None
                and conn.start - current.last_activity > max_gap_s
            ):
                flow = None
                reason = (
                    f"the client was silent for {conn.start - current.last_activity:.3f} s, "
                    f"more than max_gap_s {max_gap_s:g}"
                )
            if flow is None or current is None:
                current = Instance(len(instances) + 1, graph, client, opened_by=reason)
                instances.append(current)
                flow = next((f for f in candidates if _has_room(current, f)), candidates[0])
            current.members.append((flow.id, conn))
    return instances


def _numbered(instances: list[Instance]) -> None:
    per_scenario: dict[str, int] = defaultdict(int)
    for instance in instances:
        per_scenario[instance.graph.name] += 1
        instance.number = per_scenario[instance.graph.name]


def judge(instance: Instance, tolerance: float) -> None:
    """Compare one instance with the graph and record each broken rule in ``failures``."""
    graph = instance.graph
    where = instance.label
    for flow in graph.flows:
        found = instance.count(flow.id)
        if flow.multiplicity.allows(found):
            continue
        text = (
            f"{where}: flow {flow.id} has {found} connections, the scenario allows "
            f"{flow.multiplicity.text()}"
        )
        blamed = [e.text() for e in graph.edges if flow.id in (e.source, e.target)]
        if blamed and found < flow.multiplicity.low:
            text += f" (missing for edge {', '.join(blamed)})"
        if instance.opened_by:
            text += f"; this instance opened because {instance.opened_by}"
        instance.failures.append(text)
    for edge in graph.edges:
        before = instance.connections(edge.source)
        later = instance.connections(edge.target)
        if not before or not later:
            continue
        if edge.kind == "after":
            _judge_after(instance, edge, before, later, tolerance)
        else:
            _judge_spawn(instance, edge, before, later, tolerance)


def _judge_after(
    instance: Instance,
    edge: Edge,
    earlier: list[Connection],
    later: list[Connection],
    tolerance: float,
) -> None:
    for late in later:
        for early in earlier:
            if late.start + tolerance < early.end:
                instance.failures.append(
                    f"{instance.label}: edge {edge.text()} violated: {edge.target} "
                    f"({late.describe()}) starts at {instance.at(late.start)}, before "
                    f"{edge.source} ({early.describe()}) completes at {instance.at(early.end)}"
                )
                return


def _judge_spawn(
    instance: Instance,
    edge: Edge,
    parents: list[Connection],
    children: list[Connection],
    tolerance: float,
) -> None:
    for child in children:
        if any(p.start - tolerance <= child.start <= p.last + tolerance for p in parents):
            continue
        first = parents[0]
        if child.start < first.start - tolerance:
            when = f"before {edge.source} starts at {instance.at(first.start)}"
        else:
            when = f"after {edge.source} ended at {instance.at(max(p.last for p in parents))}"
        instance.failures.append(
            f"{instance.label}: edge {edge.text()} violated: {edge.target} "
            f"({child.describe()}) starts at {instance.at(child.start)}, {when}"
        )


def _matches(spec: StructureScenario | None, session: Session) -> bool:
    rule: Rule | None = spec.match if spec else None
    return rule is None or rule.matches(session)


def check(
    capture: Path,
    sessions: Sequence[Session],
    labels: Labels,
    scenario_files: Sequence[Path] = (),
) -> StructureResult:
    """Rebuild the scenario instances of a capture and compare them with the scenarios.

    The scenarios come from the ``structure`` section of the labels, whose files are
    relative to the labels file, and from ``scenario_files``, which the command line
    names. A scenario file that cannot be read raises
    :class:`realism.scenario_graph.ScenarioError`.
    """
    settings = labels.structure or StructureLabels()
    base = labels.base_dir or Path.cwd()
    entries: list[tuple[ScenarioGraph, StructureScenario | None]] = []
    for scenario in settings.scenarios:
        entries.append((scenario_graph.load(base / scenario.file, scenario.vars), scenario))
    for path in scenario_files:
        entries.append((scenario_graph.load(path), None))
    table = connections(capture, sessions)
    claimed: dict[int, list[Connection]] = defaultdict(list)
    for session in sorted(sessions, key=lambda s: (s.first_seen, s.id)):
        for index, (graph, spec) in enumerate(entries):
            if _matches(spec, session) and _candidates(graph, session):
                claimed[index].append(table[session.id])
                break
    results: list[ScenarioResult] = []
    failures: list[str] = []
    assigned = 0
    for index, (graph, spec) in enumerate(entries):
        mine = claimed[index]
        assigned += len(mine)
        instances = build_instances(graph, mine, settings.max_gap_s)
        _numbered(instances)
        own: list[str] = []
        for instance in instances:
            judge(instance, settings.tolerance_s)
            own += instance.failures
        if not instances:
            own.append(f"scenario {graph.name}: no session of the capture matches a flow of it")
        elif (
            spec is not None
            and spec.expected_instances is not None
            and len(instances) != spec.expected_instances
        ):
            own.append(
                f"scenario {graph.name}: {len(instances)} instances found, "
                f"expected {spec.expected_instances}"
            )
        results.append(ScenarioResult(graph, spec, instances, len(mine), own))
        failures += own
    return StructureResult(
        max_gap_s=settings.max_gap_s,
        tolerance_s=settings.tolerance_s,
        sessions_total=len(sessions),
        sessions_unassigned=len(sessions) - assigned,
        scenarios=results,
        failures=failures,
    )
