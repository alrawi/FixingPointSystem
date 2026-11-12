# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of the mix inputs of the Gate 2 suite. They need no root and no network."""

from __future__ import annotations

import importlib
import sys
from pathlib import Path
from typing import Any

import mix_config as m
import pytest

ROOT = Path(__file__).resolve().parents[3]
EVERY_MODULE = {
    "dns",
    "ftp",
    "hls",
    "http1",
    "http2",
    "imap",
    "ldap",
    "mqtt",
    "ntp",
    "pop3",
    "sip_rtp",
    "smb2",
    "smtp",
    "ssh",
}


@pytest.fixture(autouse=True)
def _need_yaml() -> None:
    pytest.importorskip("yaml")


def doc() -> dict[str, Any]:
    return m.load_mix()


def test_weights_add_up_to_100() -> None:
    assert m.weights_total(doc()) == pytest.approx(100.0)


def test_the_mix_covers_every_module_and_all_15_applications() -> None:
    entries = doc()["entries"]
    assert {e["module"] for e in entries} == EVERY_MODULE
    assert len(entries) == 15
    # HTTPS is the https_fetch entry of http1, and HLS shares the http row with http1.
    assert any(e["scenario"] == "https_fetch" for e in entries)


def test_no_two_entries_share_a_transport_and_port() -> None:
    seen = m.listen_ports(doc(), [])
    assert len(seen) == len(set(seen))


def test_waiver_ends_when_the_agent_gains_the_crate(tmp_path: Path) -> None:
    cargo = tmp_path / "Cargo.toml"
    cargo.write_text(
        'fps-module-dns = { path = "x" }\nfps-module-sip-rtp = { path = "y" }\n'
    )
    supported = m.supported_modules(cargo, EVERY_MODULE)
    assert supported == {"dns", "sip_rtp"}
    assert m.waived(["dns", "ntp", "hls"], supported) == ["hls", "ntp"]


def test_the_real_agent_crate_list_is_readable() -> None:
    supported = m.supported_modules(
        ROOT / "engine" / "fps-agent" / "Cargo.toml", EVERY_MODULE
    )
    assert {"http1", "http2", "smtp", "sip_rtp"} <= supported


def test_config_has_the_byte_basis_and_a_throughput_objective() -> None:
    text = m.test_config(doc(), "client", "agent-a", 8_000_000, 10, 120, 10, 7, [])
    config = m.yaml_module().safe_load(text)
    assert config["profile"]["basis"] == "bytes"
    assert config["load"]["objective"] == {"type": "throughput_bps", "value": 8_000_000}
    assert len(config["profile"]["mix"]) == 15
    assert config["network"]["ports"][0]["location"] == "agent-a/eth0"
    assert [p["name"] for p in config["load"]["phases"]] == [
        "ramp_up",
        "steady",
        "ramp_down",
    ]


def test_waived_modules_leave_the_config_and_the_labels() -> None:
    config = m.yaml_module().safe_load(
        m.test_config(
            doc(), "server", "a", 1, 1, 1, 1, 7, ["hls", "ntp", "ldap", "mqtt"]
        )
    )
    refs = {e["scenario"] for e in config["profile"]["mix"]}
    assert "hls/vod_ladder" not in refs and len(refs) == 11
    labels = m.yaml_module().safe_load(m.labels(doc(), ["hls", "ntp", "ldap", "mqtt"]))
    assert "hls/vod_ladder" not in labels["mix"]["scenarios"]
    assert all(r["match"]["server_port"] != 8090 for r in labels["sessions"])


def test_labels_load_in_the_realism_runner(tmp_path: Path) -> None:
    sys.path.insert(0, str(ROOT))
    realism_labels: Any = importlib.import_module("realism.labels")

    path = tmp_path / "expected.yaml"
    path.write_text(m.labels(doc(), []), encoding="utf-8")
    loaded = realism_labels.load(path)
    assert loaded.default == "aux"
    assert loaded.mix is not None and loaded.mix.tolerance_points == 2
    assert loaded.mix.scenarios["http1/https_fetch"].application == "https"
    assert loaded.mix.scenarios["hls/vod_ladder"].application == "http"
    assert loaded.accepted_names("voip", "ndpi") == ("SIP", "RTP")


def test_the_stage_puts_required_modules_first_and_sets_the_estimates(
    tmp_path: Path,
) -> None:
    order = m.stage_modules(ROOT, tmp_path, doc(), [])
    assert order.index("content") < order.index("smtp")
    assert order.index("http1") < order.index("hls")
    manifest = m.yaml_module().safe_load(
        (tmp_path / "modules" / "hls" / "manifest.yaml").read_text()
    )
    assert manifest["expected_bytes"]["vod_ladder"] == 100000
    assert manifest["expected_bytes"]["live_refresh"] == 9610000
    ssh = m.yaml_module().safe_load(
        (tmp_path / "modules" / "ssh" / "manifest.yaml").read_text()
    )
    want = next(e for e in doc()["entries"] if e["module"] == "ssh")["expected_bytes"]
    assert ssh["expected_bytes"] == {"remote_command": want}
    for name in order:
        assert (tmp_path / f"{name}.tar.gz").stat().st_size > 0


def test_a_missing_key_is_an_error(tmp_path: Path) -> None:
    bad = tmp_path / "mix.yaml"
    bad.write_text("aux: {}\nentries:\n  - {module: dns, scenario: lookup_a}\n")
    with pytest.raises(m.MixError):
        m.load_mix(bad)


def test_two_entries_on_one_port_are_an_error(tmp_path: Path) -> None:
    entry = (
        "{module: dns, scenario: lookup_a, weight: 1, expected_bytes: 1, params: {}, "
        "application: dns, ndpi: [DNS], ports: [{transport: udp, port: 53}]}"
    )
    bad = tmp_path / "mix.yaml"
    bad.write_text(f"aux: {{}}\nentries:\n  - {entry}\n  - {entry}\n")
    with pytest.raises(m.MixError):
        m.load_mix(bad)


def test_cli_prints_the_ports_and_the_names(capsys: pytest.CaptureFixture[str]) -> None:
    assert m.main(["ports", "--waived", "hls ntp ldap mqtt"]) == 0
    ports = capsys.readouterr().out.split("\n")
    assert "tcp 80" in ports and "udp 5060" in ports and "tcp 8090" not in ports
    assert m.main(["names", "--waived", "hls ntp ldap mqtt"]) == 0
    assert "hls" not in capsys.readouterr().out.split()


def test_every_param_is_a_variable_of_its_scenario() -> None:
    root = Path(__file__).resolve().parents[3]
    for e in doc()["entries"]:
        scenario = (
            root / "modules" / e["module"] / "scenarios" / f"{e['scenario']}.yaml"
        )
        declared = (m.yaml_module().safe_load(scenario.read_text()) or {}).get(
            "vars", {}
        )
        unknown = sorted(set(e["params"]) - set(declared))
        assert not unknown, (
            f"{m.scenario_ref(e)} sets {unknown}, which it does not declare"
        )


def test_the_session_basis_carries_the_byte_weights() -> None:
    mix = doc()
    config = m.yaml_module().safe_load(
        m.test_config(mix, "client", "a", 8_000_000, 10, 120, 10, 7, [], "sessions")
    )
    assert config["profile"]["basis"] == "sessions"
    rows = config["profile"]["mix"]
    assert abs(sum(e["weight"] for e in rows) - 1) < 1e-6
    estimate = {m.scenario_ref(e): e["expected_bytes"] for e in mix["entries"]}
    weight = {m.scenario_ref(e): e["weight"] for e in mix["entries"]}
    mean = sum(e["weight"] * estimate[e["scenario"]] for e in rows)
    for e in rows:
        carried = e["weight"] * estimate[e["scenario"]] / mean * 100
        assert abs(carried - weight[e["scenario"]]) < 0.01
    objective = config["load"]["objective"]
    assert objective["type"] == "sessions_per_second"
    assert abs(objective["value"] * 8 * mean - 8_000_000) < 1
