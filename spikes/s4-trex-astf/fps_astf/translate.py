#!/usr/bin/env python3
"""Spike S4: translate a FixingPointSystem test into a TRex ASTF profile.

The translator simulates both sides of every flow to learn the exact byte
counts that each `expect` consumes, because ASTF `recv` waits on a byte
watermark and cannot match patterns. It then emits an ASTF Python profile and
a report that classifies every action as exact, approximate, or unsupported.

The emitted profile targets the ASTF API on the TRex master branch at commit 27e0153 (VERSION v3.08).
"""
from __future__ import annotations

import argparse
import ipaddress
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import yaml

FILL_BYTE = b"x"
MAX_UNROLLED_STEPS = 100_000

# --------------------------------------------------------------------------
# Template rendering (contract v0.1 core grammar)
# --------------------------------------------------------------------------

PLACEHOLDER = re.compile(r"\{\{(.*?)\}\}", re.S)


class NonDeterministic(Exception):
    """The template depends on per session randomness or identity."""


@dataclass
class Rendered:
    data: bytes
    fill: int = 0  # trailing bytes produced by fill()


def _arg(text: str, vars_: dict) -> Any:
    text = text.strip()
    if re.fullmatch(r"-?\d+", text):
        return int(text)
    if len(text) >= 2 and text[0] == text[-1] == "'":
        return text[1:-1]
    if text.startswith("vars."):
        return vars_[text[5:]]
    if text.startswith("session."):
        raise NonDeterministic(text)
    raise ValueError(f"cannot evaluate argument '{text}'")


def render(template: Any, vars_: dict) -> Rendered:
    if not isinstance(template, str):
        return Rendered(str(template).encode())
    out = bytearray()
    fill = 0
    pos = 0
    for m in PLACEHOLDER.finditer(template):
        lit = template[pos:m.start()].encode()
        if lit and fill:
            raise NonDeterministic("literal text after fill() is not representable in ASTF send")
        out += lit
        pos = m.end()
        expr = m.group(1).strip()
        call = re.fullmatch(r"([a-z_.]+)\s*\((.*)\)", expr, re.S)
        if call:
            name, raw = call.group(1), call.group(2)
            args = [a for a in re.split(r",(?=(?:[^']*'[^']*')*[^']*$)", raw) if a.strip()]
            if name == "fill":
                n = int(_arg(args[0], vars_))
                out += FILL_BYTE * n
                fill += n
            elif name == "hex":
                if fill:
                    raise NonDeterministic("literal bytes after fill() are not representable in ASTF send")
                out += bytes.fromhex(_arg(args[0], vars_))
            elif name == "rand_int":
                raise NonDeterministic("rand_int")
            else:
                raise NonDeterministic(f"helper {name}")
        else:
            value = _arg(expr, vars_)
            if fill:
                raise NonDeterministic("literal text after fill() is not representable in ASTF send")
            out += str(value).encode()
    tail = template[pos:].encode()
    if tail and fill:
        raise NonDeterministic("literal text after fill() is not representable in ASTF send")
    out += tail
    return Rendered(bytes(out), fill)


def render_int(template: Any, vars_: dict) -> int:
    return int(render(template, vars_).data.decode().strip())


# --------------------------------------------------------------------------
# Report
# --------------------------------------------------------------------------

@dataclass
class Report:
    entries: list[dict] = field(default_factory=list)

    def add(self, scenario: str, flow: str, side: str, action: str, verdict: str, note: str = "") -> None:
        self.entries.append(dict(scenario=scenario, flow=flow, side=side, action=action, verdict=verdict, note=note))

    def summary(self) -> dict:
        counts = {"exact": 0, "approximate": 0, "unsupported": 0}
        for e in self.entries:
            counts[e["verdict"]] += 1
        total = sum(counts.values()) or 1
        return {**counts, "total": sum(counts.values()), "exact_share": counts["exact"] / total}


# --------------------------------------------------------------------------
# Conversation simulation
# --------------------------------------------------------------------------

@dataclass
class Step:
    """One executed action instance on one side."""
    path: tuple  # position in the action tree, loop iterations excluded
    kind: str
    body: Any
    consumed: int = 0


class Unsupported(Exception):
    pass


def _unroll(actions: list, vars_: dict, path: tuple = ()) -> list[Step]:
    steps: list[Step] = []
    for i, action in enumerate(actions):
        (kind, body), = action.items()
        here = path + (i,)
        if kind == "loop":
            n = render_int(body["count"], vars_)
            for _ in range(n):
                steps.extend(_unroll(body["do"], vars_, here))
        elif kind in ("branch", "spawn", "raw", "set"):
            raise Unsupported(kind)
        else:
            steps.append(Step(here, kind, body))
        if len(steps) > MAX_UNROLLED_STEPS:
            raise Unsupported("program too long to simulate")
    return steps


def simulate(client: list, server: list, vars_: dict) -> tuple[list[Step], list[Step]]:
    """Run both sides until completion and record bytes consumed per expect."""
    sides = {"c": _unroll(client, vars_), "s": _unroll(server, vars_)}
    pc = {"c": 0, "s": 0}
    inbox = {"c": bytearray(), "s": bytearray()}
    peer = {"c": "s", "s": "c"}
    closed = {"c": False, "s": False}

    def run(side: str) -> bool:
        progressed = False
        steps = sides[side]
        while pc[side] < len(steps):
            st = steps[pc[side]]
            if st.kind in ("connect", "accept", "delay"):
                pass
            elif st.kind == "send":
                inbox[peer[side]] += render(st.body, vars_).data
            elif st.kind == "close":
                closed[side] = True
            elif st.kind == "expect":
                buf = inbox[side]
                if "pattern" in st.body:
                    pat = st.body["pattern"].encode()
                    i = buf.find(pat)
                    if i < 0:
                        return progressed
                    st.consumed = i + len(pat)
                else:
                    key = "bytes" if "bytes" in st.body else "min_bytes"
                    n = render_int(st.body[key], vars_)
                    if len(buf) < n:
                        return progressed
                    st.consumed = n if key == "bytes" else len(buf)
                del buf[:st.consumed]
            else:
                raise Unsupported(st.kind)
            pc[side] += 1
            progressed = True
        return progressed

    for _ in range(4 * (len(sides["c"]) + len(sides["s"])) + 4):
        a, b = run("c"), run("s")
        if pc["c"] >= len(sides["c"]) and pc["s"] >= len(sides["s"]):
            if inbox["c"] or inbox["s"]:
                raise Unsupported(f"unread bytes at close: client {len(inbox['c'])}, server {len(inbox['s'])}")
            return sides["c"], sides["s"]
        if not (a or b):
            raise Unsupported("the two sides deadlock: an expect can never be satisfied")
    raise Unsupported("simulation did not converge")


# --------------------------------------------------------------------------
# ASTF emission
# --------------------------------------------------------------------------

def _py_bytes(b: bytes) -> str:
    return repr(b)


class Emitter:
    def __init__(self, scenario: str, flow: str, side: str, report: Report, l4: str):
        self.lines: list[str] = []
        self.scenario, self.flow, self.side, self.report, self.l4 = scenario, flow, side, report, l4
        self.var_seq = 0
        self.pending_recv = 0
        self.in_loop = 0

    def note(self, action: str, verdict: str, note: str = "") -> None:
        self.report.add(self.scenario, self.flow, self.side, action, verdict, note)

    def flush_recv(self) -> None:
        if self.pending_recv:
            if self.l4 == "udp":
                self.lines.append("prog.recv_msg(1, clear=True)")
            else:
                # clear=True resets the watermark, which keeps loop iterations independent.
                self.lines.append(f"prog.recv({self.pending_recv}, clear=True)")
            self.pending_recv = 0

    def emit(self, actions: list, steps_by_path: dict, vars_: dict, path: tuple = ()) -> None:
        for i, action in enumerate(actions):
            (kind, body), = action.items()
            here = path + (i,)
            if kind == "expect":
                consumed = steps_by_path.get(here, set())
                if len(consumed) != 1:
                    self.flush_recv()
                    self.note(kind, "unsupported", f"consumed byte count varies between iterations: {sorted(consumed)}")
                    continue
                self.pending_recv += next(iter(consumed))
                rule = next(k for k in ("pattern", "bytes", "min_bytes") if k in body)
                if rule == "bytes":
                    self.note(kind, "exact")
                else:
                    self.note(kind, "approximate", f"{rule} becomes a fixed byte watermark learned by simulation")
                continue
            self.flush_recv()
            if kind in ("connect", "accept"):
                if self.l4 == "tcp":
                    self.lines.append(f"prog.{kind}()")
                self.note(kind, "exact")
            elif kind == "send":
                r = render(body, vars_)
                head = r.data[: len(r.data) - r.fill]
                call = "send_msg" if self.l4 == "udp" else "send"
                if r.fill:
                    self.lines.append(f"prog.{call}({_py_bytes(head)}, size={len(r.data)}, fill={_py_bytes(FILL_BYTE)})")
                    self.note(kind, "approximate", "fill() content becomes one repeated byte in ASTF")
                else:
                    self.lines.append(f"prog.{call}({_py_bytes(head)})")
                    self.note(kind, "exact")
            elif kind == "delay":
                if "ms" in body:
                    self.lines.append(f"prog.delay({render_int(body['ms'], vars_) * 1000})")
                    self.note(kind, "exact")
                elif body.get("dist") == "uniform":
                    self.lines.append(f"prog.delay_rand({int(body['min_ms'] * 1000)}, {int(body['max_ms'] * 1000)})")
                    self.note(kind, "exact")
                else:
                    self.note(kind, "unsupported", f"ASTF has fixed and uniform delays only, not {body.get('dist')}")
            elif kind == "loop":
                self.var_seq += 1
                var, label = f"var{self.var_seq}", f"loop{self.var_seq}:"
                n = render_int(body["count"], vars_)
                self.lines.append(f"prog.set_var({var!r}, {n})")
                self.lines.append(f"prog.set_label({label!r})")
                self.in_loop += 1
                self.emit(body["do"], steps_by_path, vars_, here)
                self.flush_recv()
                self.in_loop -= 1
                self.lines.append(f"prog.jmp_nz({var!r}, {label!r})")
                self.note(kind, "exact")
            elif kind == "close":
                if self.l4 == "udp":
                    self.note(kind, "exact", "UDP flows end when the program ends")
                elif body == "rst":
                    self.lines.append("prog.reset()")
                    self.note(kind, "exact")
                elif body == "fin":
                    if self.side == "server":
                        self.lines.append("prog.wait_for_peer_close()")
                    self.note(kind, "exact", "ASTF closes with FIN when the program ends")
                else:
                    self.note(kind, "unsupported", "ASTF has no half close")
            else:
                self.note(kind, "unsupported", f"ASTF has no equivalent of '{kind}'")
        self.flush_recv()


@dataclass
class TemplatePlan:
    scenario: str
    flow: str
    port: int
    l4: str
    weight: float
    client_lines: list[str]
    server_lines: list[str]
    bytes_per_session: int


def plan_flow(scenario: dict, flow: dict, vars_: dict, weight: float, report: Report) -> TemplatePlan | None:
    sname, fid = scenario["scenario"], flow["id"]
    l4 = flow.get("l4", "tcp")
    for key in ("transport", "spawned"):
        if flow.get(key):
            report.add(sname, fid, "flow", key, "unsupported", f"ASTF cannot express '{key}'")
            return None
    if flow.get("after"):
        report.add(sname, fid, "flow", "after", "unsupported", "ASTF templates run independently, so flow order is lost")
    if "server" not in flow:
        report.add(sname, fid, "flow", "server", "unsupported", "client simulation against real servers is outside this spike")
        return None
    try:
        csteps, ssteps = simulate(flow["client"], flow["server"], vars_)
    except (Unsupported, NonDeterministic) as exc:
        report.add(sname, fid, "flow", "simulate", "unsupported", str(exc))
        return None
    by_path: dict[str, dict[tuple, set]] = {"client": {}, "server": {}}
    for side, steps in (("client", csteps), ("server", ssteps)):
        for st in steps:
            if st.kind == "expect":
                by_path[side].setdefault(st.path, set()).add(st.consumed)
    total = sum(len(render(s.body, vars_).data) for s in csteps + ssteps if s.kind == "send")
    out = {}
    for side in ("client", "server"):
        em = Emitter(sname, fid, side, report, l4)
        em.emit(flow[side], by_path[side], vars_)
        out[side] = em.lines
    port = render_int(flow["client"][0]["connect"]["port"], vars_)
    return TemplatePlan(sname, fid, port, l4, weight, out["client"], out["server"], total)


# --------------------------------------------------------------------------
# Test level translation
# --------------------------------------------------------------------------

def load_modules(root: Path) -> dict[str, dict[str, dict]]:
    index: dict[str, dict[str, dict]] = {}
    for mod_dir in sorted(p for p in root.iterdir() if p.is_dir()):
        manifest = yaml.safe_load((mod_dir / "manifest.yaml").read_text())
        for rel in manifest.get("scenarios", []):
            doc = yaml.safe_load((mod_dir / rel).read_text())
            index.setdefault(manifest["name"], {})[doc["scenario"]] = doc
    return index


def resolve(index: dict, ref: str) -> dict:
    if "/" in ref:
        mod, name = ref.split("/", 1)
        return index[mod][name]
    hits = [s[ref] for s in index.values() if ref in s]
    if len(hits) != 1:
        raise KeyError(f"scenario '{ref}' is missing or ambiguous")
    return hits[0]


def ip_range(pool: dict) -> tuple[str, str]:
    net = ipaddress.ip_network(pool["ipv4"], strict=False)
    hosts = list(net.hosts()) if net.prefixlen < 31 else list(net)
    start = 1 if pool.get("gateway") == str(hosts[0]) else 0
    chosen = hosts[start:start + pool["count"]]
    return str(chosen[0]), str(chosen[-1])


def server_addresses(pool: dict) -> list:
    net = ipaddress.ip_network(pool["ipv4"], strict=False)
    hosts = list(net.hosts()) if net.prefixlen < 31 else list(net)
    start = 1 if pool.get("gateway") == str(hosts[0]) else 0
    return hosts[start:start + pool["count"]]


def translate(test: dict, index: dict) -> tuple[str, Report, dict]:
    report = Report()
    pools = {p["name"]: p for p in test["network"]["host_pools"]}
    client_pool = next(p for p in pools.values() if p["role"] == "client")
    server_pool = next(p for p in pools.values() if p["role"] == "server")
    plans: list[TemplatePlan] = []
    for entry in test["profile"]["mix"]:
        scenario = resolve(index, entry["scenario"])
        vars_ = dict(scenario.get("vars") or {})
        vars_.update(entry.get("params") or {})
        flows = scenario["flows"]
        for flow in flows:
            plan = plan_flow(scenario, flow, vars_, entry["weight"] / len(flows), report)
            if plan:
                plans.append(plan)

    load = test["load"]["objective"]
    basis = test["profile"]["basis"]
    meta: dict[str, Any] = {"objective": load["type"], "basis": basis}
    # Share of sessions per plan. A byte basis divides each weight by bytes per session.
    raw = [p.weight / max(p.bytes_per_session, 1) if basis == "bytes" else p.weight for p in plans]
    total_raw = sum(raw) or 1.0
    session_share = [r / total_raw for r in raw]
    mean_bytes = sum(sh * p.bytes_per_session for sh, p in zip(session_share, plans))
    if load["type"] == "sessions_per_second":
        total_cps = float(load["value"])
    elif load["type"] == "throughput_bps":
        total_cps = load["value"] / 8.0 / max(mean_bytes, 1)
        report.add("*", "*", "load", "objective", "approximate",
                   f"throughput becomes {total_cps:.1f} sessions/s from {mean_bytes:.0f} payload bytes per session; "
                   "headers below L7 are not counted")
    else:
        total_cps = float(load["value"])
        report.add("*", "*", "load", "objective", "unsupported", f"ASTF schedules by CPS, not by {load['type']}")
    if len(test["load"]["phases"]) > 1 or any(p.get("shape", "hold") != "hold" for p in test["load"]["phases"]):
        report.add("*", "*", "load", "phases", "approximate", "ramps need runtime multiplier updates from the driver")
    meta["total_cps"] = total_cps
    meta["mean_payload_bytes_per_session"] = mean_bytes

    c0, c1 = ip_range(client_pool)
    servers = server_addresses(server_pool)
    # Templates that share a port need disjoint server address ranges, because
    # ASTF associates server programs by port and destination address.
    ports = [p.port for p in plans]
    shared_port = len(set(ports)) < len(ports)
    if shared_port and len(servers) < len(plans):
        report.add("*", "*", "flow", "port", "unsupported",
                   f"{len(plans)} templates share ports but the server pool has only {len(servers)} addresses")
    chunk = max(len(servers) // max(len(plans), 1), 1)
    lines = [
        '"""Generated by FixingPointSystem spike S4. Do not edit."""',
        "from trex.astf.api import *",
        "",
        "",
        "class Profile:",
        "    def get_profile(self, tunables=None, **kwargs):",
        "        templates = []",
    ]
    for n, (p, share) in enumerate(zip(plans, session_share)):
        if shared_port and len(servers) >= len(plans):
            s0, s1 = str(servers[n * chunk]), str(servers[(n + 1) * chunk - 1])
            assoc = f"ASTFAssociationRule(port={p.port}, ip_start={s0!r}, ip_end={s1!r})"
        else:
            s0, s1 = str(servers[0]), str(servers[-1])
            assoc = f"ASTFAssociationRule(port={p.port})"
        cps = total_cps * share
        lines.append(f"        # {p.scenario}/{p.flow}: {p.bytes_per_session} payload bytes per session")
        lines.append(f"        ip_gen = ASTFIPGen(glob=ASTFIPGenGlobal(ip_offset='1.0.0.0'),")
        lines.append(f"                           dist_client=ASTFIPGenDist(ip_range=[{c0!r}, {c1!r}], distribution='seq'),")
        lines.append(f"                           dist_server=ASTFIPGenDist(ip_range=[{s0!r}, {s1!r}], distribution='seq'))")
        for side, body in (("c", p.client_lines), ("s", p.server_lines)):
            udp = ", stream=False" if p.l4 == "udp" else ""
            lines.append(f"        prog = ASTFProgram(side={side!r}{udp})")
            lines.extend("        " + ln for ln in body)
            lines.append(f"        prog_{side}{n} = prog")
        lines.append("        templates.append(ASTFTemplate(")
        lines.append(f"            client_template=ASTFTCPClientTemplate(program=prog_c{n}, ip_gen=ip_gen, port={p.port}, cps={cps:.3f}),")
        lines.append(f"            server_template=ASTFTCPServerTemplate(program=prog_s{n}, assoc={assoc}),")
        lines.append(f"            tg_name={(p.scenario + '_' + p.flow)[:20]!r}))")
    lines += [
        "        return ASTFProfile(default_ip_gen=ip_gen, templates=templates)",
        "",
        "",
        "def register():",
        "    return Profile()",
        "",
    ]
    return "\n".join(lines), report, meta


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--test", type=Path, required=True)
    ap.add_argument("--modules", type=Path, required=True)
    ap.add_argument("--out", type=Path, help="write the ASTF profile here")
    ap.add_argument("--report", type=Path, help="write the translation report as JSON here")
    args = ap.parse_args(argv)
    test = yaml.safe_load(args.test.read_text())
    profile, report, meta = translate(test, load_modules(args.modules))
    if args.out:
        args.out.write_text(profile)
    else:
        print(profile)
    summary = report.summary()
    for e in report.entries:
        if e["verdict"] != "exact":
            print(f"{e['verdict']:12} {e['scenario']}/{e['flow']} {e['side']} {e['action']}: {e['note']}", file=sys.stderr)
    print(f"actions: {summary['exact']} exact, {summary['approximate']} approximate, "
          f"{summary['unsupported']} unsupported ({summary['exact_share']:.0%} exact)", file=sys.stderr)
    if args.report:
        args.report.write_text(json.dumps({"meta": meta, "summary": summary, "entries": report.entries}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
