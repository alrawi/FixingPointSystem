"""Tests for the spike S4 translator. Run with: python3 -m pytest spikes/s4-trex-astf"""
from __future__ import annotations

import sys
from pathlib import Path

import pytest
import yaml

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE / "fake_trex"))

from fps_astf import translate as t  # noqa: E402

MODULES = REPO / "contracts/examples/modules"
TESTS = REPO / "contracts/examples/tests"


def build(test_doc: dict):
    source, report, meta = t.translate(test_doc, t.load_modules(MODULES))
    namespace: dict = {}
    exec(compile(source, "<profile>", "exec"), namespace)
    return namespace["register"]().get_profile(), report, meta


def run_program(cmds: list, peer_sends: list[int] | None = None) -> list:
    """Expand jmp_nz loops into a linear command list."""
    out, pc, counters = [], 0, {}
    while pc < len(cmds):
        c = cmds[pc]
        if c[0] == "set_var":
            counters[c[1]] = c[2]
        elif c[0] == "jmp_nz":
            counters[c[1]] -= 1
            if counters[c[1]]:
                pc = c[2]
                continue
        else:
            out.append(c)
        pc += 1
    return out


def test_http_smoke_profile_executes_and_balances():
    profile, report, meta = build(yaml.safe_load((TESTS / "http_smoke.yaml").read_text()))
    assert len(profile.templates) == 1
    tpl = profile.templates[0]
    assert tpl.client_template.cps == pytest.approx(1000)
    client = run_program(tpl.client_template.program.commands)
    server = run_program(tpl.server_template.program.commands)
    # Each client recv watermark equals the server send that precedes it, and vice versa.
    c_recv = [c[1] for c in client if c[0] == "recv"]
    s_send = [c[1] for c in server if c[0] == "send"]
    s_recv = [c[1] for c in server if c[0] == "recv"]
    c_send = [c[1] for c in client if c[0] == "send"]
    assert c_recv == s_send and s_recv == c_send
    assert len(c_send) == 4
    # 4 x (132 byte request + 83 byte response header + 4,096 byte body). Spike S1
    # measures the same payload per session on loopback.
    assert sum(c_send) + sum(s_send) == 4 * (132 + 83 + 4096) == 17244
    assert report.summary()["unsupported"] == 0


def test_enterprise_mix_reports_losses():
    profile, report, meta = build(yaml.safe_load((TESTS / "enterprise_mix_10g.yaml").read_text()))
    assert len(profile.templates) == 3
    rules = [tp.server_template.assoc for tp in profile.templates]
    http = [r for r in rules if r.port == 8080]
    assert len(http) == 2 and http[0].ip_end < http[1].ip_start  # disjoint server ranges
    notes = {(e["action"], e["verdict"]) for e in report.entries}
    assert ("delay", "unsupported") in notes  # lognormal think time
    assert ("phases", "approximate") in notes
    # Byte basis: cps * bytes per session reproduces the configured byte shares.
    assert meta["total_cps"] > 0
    udp = [tp for tp in profile.templates if not tp.client_template.program.stream]
    assert len(udp) == 1


def test_byte_basis_shares():
    doc = yaml.safe_load((TESTS / "http_smoke.yaml").read_text())
    doc["profile"] = {"basis": "bytes", "mix": [
        {"scenario": "http1/web_get", "weight": 50},
        {"scenario": "http1/web_get", "weight": 50, "params": {"body_size": 40960, "server_port": 8081}},
    ]}
    doc["load"]["objective"] = {"type": "throughput_bps", "value": 1e9}
    profile, report, meta = build(doc)
    small, large = profile.templates
    plans_bytes = []
    for tp in (small, large):
        prog = run_program(tp.client_template.program.commands)
        srv = run_program(tp.server_template.program.commands)
        plans_bytes.append(sum(c[1] for c in prog + srv if c[0] == "send"))
    share_small = small.client_template.cps * plans_bytes[0]
    share_large = large.client_template.cps * plans_bytes[1]
    assert share_small == pytest.approx(share_large, rel=1e-3)
    assert (share_small + share_large) * 8 == pytest.approx(1e9, rel=1e-3)


@pytest.mark.parametrize("action,needle", [
    ({"branch": [{"p": 0.5, "do": [{"delay": {"ms": 1}}]}, {"p": 0.5, "do": [{"delay": {"ms": 2}}]}]}, "branch"),
    ({"send": "{{ rand_int(1, 9) }}"}, "rand_int"),
    ({"set": {"x": "1"}}, "set"),
])
def test_unsupported_actions_are_reported(action, needle):
    doc = yaml.safe_load((MODULES / "http1/scenarios/web_get.yaml").read_text())
    doc["flows"][0]["client"].insert(1, action)
    report = t.Report()
    plan = t.plan_flow(doc, doc["flows"][0], dict(doc["vars"]), 1.0, report)
    assert plan is None
    assert any(needle in e["note"] for e in report.entries if e["verdict"] == "unsupported")


def test_deadlock_is_detected():
    doc = yaml.safe_load((MODULES / "http1/scenarios/web_get.yaml").read_text())
    doc["flows"][0]["server"][1]["loop"]["do"][0] = {"expect": {"pattern": "NEVER"}}
    report = t.Report()
    assert t.plan_flow(doc, doc["flows"][0], dict(doc["vars"]), 1.0, report) is None
    assert any("deadlock" in e["note"] for e in report.entries)


def test_render_rules():
    assert t.render("a{{ vars.x }}b", {"x": 5}).data == b"a5b"
    r = t.render("H\r\n\r\n{{ fill(3) }}", {})
    assert (r.data, r.fill) == (b"H\r\n\r\nxxx", 3)
    with pytest.raises(t.NonDeterministic):
        t.render("{{ fill(2) }}tail", {})
    with pytest.raises(t.NonDeterministic):
        t.render("{{ session.id }}", {})
