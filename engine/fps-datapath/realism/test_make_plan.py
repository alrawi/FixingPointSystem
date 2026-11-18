# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of make_plan.py and profiles.yaml, which need no root and no Rust toolchain."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT))

import make_plan

from realism import labels as realism_labels

# Modules without scenarios: content is a function library and tls is the TLS
# library of the other modules, so neither has a session to record.
LIBRARIES = {"content", "tls", "_template"}


def modules() -> list[str]:
    return sorted(
        p.name
        for p in (ROOT / "modules").iterdir()
        if p.is_dir() and p.name not in LIBRARIES
    )


def plans(out: Path) -> list[dict[str, Any]]:
    count = int((out / "rounds").read_text(encoding="utf-8"))
    return [
        json.loads((out / f"round-{i}" / "plan.json").read_text(encoding="utf-8"))
        for i in range(count)
    ]


@pytest.fixture(scope="module")
def http1(tmp_path_factory: pytest.TempPathFactory) -> Path:
    out = tmp_path_factory.mktemp("http1")
    make_plan.build(ROOT, out, "http1", None, 42, "dpc", "dps", True)
    return out


def test_every_first_party_module_has_a_profile() -> None:
    profiles = yaml.safe_load((HERE / "profiles.yaml").read_text(encoding="utf-8"))
    assert sorted(profiles) == modules()


@pytest.mark.parametrize("module", modules())
def test_a_round_never_listens_twice_on_one_port(
    module: str, tmp_path: Path
) -> None:
    rounds = make_plan.build(ROOT, tmp_path, module, None, 42, "dpc", "dps", True)
    manifest = yaml.safe_load(
        (ROOT / "modules" / module / "manifest.yaml").read_text(encoding="utf-8")
    )
    profile = make_plan.load_profile(module)
    wanted = {
        yaml.safe_load((ROOT / "modules" / module / rel).read_text(encoding="utf-8"))[
            "scenario"
        ]
        for rel in manifest["scenarios"]
    } - set(profile.get("skip", [])) - set(profile.get("excluded", {}))
    seen: list[str] = []
    for plan in plans(tmp_path):
        assert plan["backend"] == "BACKEND_AF_XDP"
        listens = []
        for entry in plan["mix"]:
            doc = yaml.safe_load(
                next(
                    s["source"]
                    for s in plan["scenarios"]
                    if s["ref"] == entry["scenario"]
                )
            )
            listens.append(make_plan.entry_port(doc, entry["params"]))
            seen.append(entry["id"])
        assert len(listens) == len(set(listens)), f"{module}: {listens}"
    assert len(plans(tmp_path)) == rounds
    assert sorted(seen) == sorted(wanted), "every scenario runs in exactly one round"


@pytest.mark.parametrize("module", modules())
def test_the_variables_of_a_profile_are_declared_by_a_scenario(module: str) -> None:
    profile = make_plan.load_profile(module)
    manifest = yaml.safe_load(
        (ROOT / "modules" / module / "manifest.yaml").read_text(encoding="utf-8")
    )
    docs = {
        d["scenario"]: d
        for d in (
            yaml.safe_load((ROOT / "modules" / module / rel).read_text(encoding="utf-8"))
            for rel in manifest["scenarios"]
        )
    }
    declared = {k for d in docs.values() for k in d.get("vars", {})}
    for key in profile.get("vars", {}):
        assert key in declared, f"{module}: no scenario declares {key}"
    for name, values in profile.get("scenario_vars", {}).items():
        assert name in docs, f"{module}: no scenario {name}"
        # A scenario takes only the variables that it declares, as with --var-tls.
        for key in values:
            assert key in declared, f"{module}/{name}: no scenario declares {key}"
    for name in [*profile.get("skip", []), *profile.get("ports", {})]:
        assert name in docs, f"{module}: no scenario {name}"


def test_http1_runs_its_plain_scenarios_in_rounds_on_the_ports_of_the_kernel_run(
    http1: Path,
) -> None:
    by_port: dict[int, list[str]] = {}
    for plan in plans(http1):
        for entry in plan["mix"]:
            doc = yaml.safe_load(
                next(
                    s["source"]
                    for s in plan["scenarios"]
                    if s["ref"] == entry["scenario"]
                )
            )
            _, port = make_plan.entry_port(doc, entry["params"])
            by_port.setdefault(port, []).append(entry["id"])
    assert sorted(by_port) == [8080, 8443, 8444, 8445, 8446]
    assert len(by_port[8080]) == 6
    assert (http1 / "rounds").read_text(encoding="utf-8") == "6\n"


def test_every_round_has_its_own_client_addresses_and_seed(http1: Path) -> None:
    all_plans = plans(http1)
    prefixes = [p["hostPools"][0]["ipv4Prefix"] for p in all_plans]
    assert len(set(prefixes)) == len(prefixes)
    assert len({p["seed"] for p in all_plans}) == len(all_plans)
    assert len({p["hostPools"][0]["macBase"] for p in all_plans}) == len(all_plans)


def test_the_rates_give_the_session_count_of_the_profile(http1: Path) -> None:
    for plan in plans(http1):
        seconds = int(plan["load"]["phases"][0]["duration"].rstrip("s"))
        for rate in plan["workers"][0]["rates"]:
            assert round(rate["sessionsPerSecond"] * seconds) == 20


def test_the_module_tree_digest_covers_only_the_staged_files(http1: Path) -> None:
    plan = plans(http1)[0]
    tree = http1 / "modules" / "http1"
    assert sorted(p.name for p in tree.iterdir()) == ["manifest.yaml", "scenarios"]
    assert plan["modules"][0]["digest"] == make_plan.tree_digest(tree)
    assert plan["modules"][0]["digest"].startswith("sha256:")


def test_the_data_files_of_a_module_stay_in_the_tree(tmp_path: Path) -> None:
    make_plan.build(ROOT, tmp_path, "dns", None, 42, "dpc", "dps", True)
    assert (tmp_path / "modules" / "dns" / "zones" / "example.test.zone").is_file()
    assert not (tmp_path / "modules" / "dns" / "native").exists()


def test_the_addresses_in_the_payload_are_the_hosts_of_the_round(
    tmp_path: Path,
) -> None:
    make_plan.build(ROOT, tmp_path, "sip_rtp", None, 42, "dpc", "dps", True)
    for index, plan in enumerate(plans(tmp_path)):
        assert plan["hostPools"][0]["count"] == 1
        for entry in plan["mix"]:
            assert entry["params"]["server_ip"] == "10.2.0.1"
            assert entry["params"]["client_ip"] == f"10.1.{index}.1"


def test_the_labels_of_the_kernel_run_load_and_keep_their_ports(http1: Path) -> None:
    labels = realism_labels.load(http1 / "labels.yaml")
    assert labels.tls is not None
    text = (http1 / "labels.yaml").read_text(encoding="utf-8")
    for port in [8080, 8443, 8444, 8445, 8446]:
        assert f"server_port: {port}" in text


def test_without_keys_the_labels_leave_the_encryption_check_out(tmp_path: Path) -> None:
    make_plan.build(ROOT, tmp_path, "http1", None, 42, "dpc", "dps", False)
    labels = realism_labels.load(tmp_path / "labels.yaml")
    assert labels.tls is None
    assert "tls_profile" not in (tmp_path / "labels.yaml").read_text(encoding="utf-8")
    assert (tmp_path / "decrypted.args").read_text(encoding="utf-8") == ""


def test_the_decrypted_check_follows_the_profile(tmp_path: Path) -> None:
    make_plan.build(ROOT, tmp_path, "imap", None, 42, "dpc", "dps", True)
    args = (tmp_path / "decrypted.args").read_text(encoding="utf-8").split("\n")
    assert args[:2] == ["--labels", "realism/examples/decrypted/imap.labels.yaml"]
    assert "--starttls-port" in args and "1143:imap" in args
    make_plan.build(ROOT, tmp_path, "dns", None, 42, "dpc", "dps", True)
    assert (tmp_path / "decrypted.args").read_text(encoding="utf-8") == ""


def test_a_module_without_a_profile_is_refused(tmp_path: Path) -> None:
    with pytest.raises(SystemExit, match="no profile"):
        make_plan.build(ROOT, tmp_path, "content", None, 42, "dpc", "dps", True)
