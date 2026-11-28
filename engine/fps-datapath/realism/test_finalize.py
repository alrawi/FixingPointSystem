# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of finalize.py, which need no root and no Rust toolchain."""

from __future__ import annotations

import json
import sys
from collections import Counter
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT))

import finalize
import make_plan


def report(path: Path, started: dict[str, int]) -> None:
    """Writes the final record of an agent run, with the counters that finalize reads."""
    counters = []
    for scenario, count in started.items():
        for side in ("client", "server"):
            counters.append(
                {
                    "count": str(count),
                    "labels": {"scenario": scenario, "side": side},
                    "metric": "sessions.attempted",
                }
            )
    early = {"counters": [], "sequence": "1"}
    last = {"counters": counters, "sequence": "2", "isFinal": True}
    path.write_text(
        json.dumps(early) + "\n" + json.dumps(last) + "\n", encoding="utf-8"
    )


def test_the_sessions_of_every_round_add_up_per_scenario(tmp_path: Path) -> None:
    report(tmp_path / "agent-0.jsonl", {"a": 3, "b": 5})
    report(tmp_path / "agent-1.jsonl", {"a": 2})
    assert finalize.started(tmp_path) == Counter({"a": 5, "b": 5})


def test_a_placeholder_sums_the_sessions_with_their_weights() -> None:
    counts = Counter({"list_pasv": 2, "mixed_session": 3, "batch_download": 1})
    values = {"pcap": "x.pcapng", "started": "6"}
    got = finalize.expand(
        "{sum:list_pasv+mixed_session*3+batch_download*4}", values, counts
    )
    assert got == str(2 + 9 + 4)
    assert finalize.expand("{pcap}", values, counts) == "x.pcapng"
    assert finalize.expand("{sum:absent*2}", values, counts) == "0"


def test_the_labels_take_the_sessions_that_the_agent_started(tmp_path: Path) -> None:
    make_plan.build(ROOT, tmp_path, "strike", None, 42, "dpc", "dps", True)
    report(tmp_path / "agent-0.jsonl", {"slowloris": 23, "conn_churn": 21})
    finalize.apply_counts(tmp_path, make_plan.load_profile("strike"))
    labels = yaml.safe_load((tmp_path / "labels.yaml").read_text(encoding="utf-8"))
    scenarios = {
        Path(s["file"]).stem: s["expected_instances"]
        for s in labels["structure"]["scenarios"]
    }
    assert scenarios == {"slowloris": 23, "conn_churn": 21}
    assert json.loads((tmp_path / "started.json").read_text(encoding="utf-8")) == {
        "conn_churn": 21,
        "slowloris": 23,
    }


def test_the_streams_of_the_rung_check_are_the_playbacks(tmp_path: Path) -> None:
    make_plan.build(ROOT, tmp_path, "hls", None, 42, "dpc", "dps", True)
    report(tmp_path / "agent-0.jsonl", {"vod_ladder": 4, "live_refresh": 2})
    finalize.apply_counts(tmp_path, make_plan.load_profile("hls"))
    labels = yaml.safe_load((tmp_path / "labels.yaml").read_text(encoding="utf-8"))
    rungs = labels["temporal"]["rungs"]["applications"]
    assert rungs["hls_vod"]["streams"] == 4
    assert rungs["hls_live"]["streams"] == 2
    assert "hls_segment" in rungs, "vod_segments runs, so its labels stay"


def test_nothing_is_excluded_so_every_label_stays(tmp_path: Path) -> None:
    # The exclusion list is empty since bug #689: dns keeps its TCP label and
    # strike keeps conn_churn.
    make_plan.build(ROOT, tmp_path, "dns", None, 42, "dpc", "dps", True)
    labels = yaml.safe_load((tmp_path / "labels.yaml").read_text(encoding="utf-8"))
    assert {"transport": "tcp", "server_port": 53} in [
        e["match"] for e in labels["sessions"]
    ]
    make_plan.build(ROOT, tmp_path, "strike", None, 42, "dpc", "dps", True)
    labels = yaml.safe_load((tmp_path / "labels.yaml").read_text(encoding="utf-8"))
    assert [e["match"]["server_port"] for e in labels["sessions"]] == [8080, 8081]
    files = [Path(e["file"]) for e in labels["structure"]["scenarios"]]
    assert [f.name for f in files] == ["slowloris.yaml", "conn_churn.yaml"]
    assert all(f.is_file() for f in files), "the file of a scenario is an absolute path"


def test_the_labels_of_an_excluded_scenario_leave_with_it(tmp_path: Path) -> None:
    # The mechanism stays for the next entry of the exclusion list.
    make_plan.build(ROOT, tmp_path, "dns", None, 42, "dpc", "dps", True)
    labels = yaml.safe_load((tmp_path / "labels.yaml").read_text(encoding="utf-8"))
    make_plan.prune_labels(
        labels, {"drop_rules": [{"transport": "tcp", "server_port": 53}]}
    )
    assert [e["match"] for e in labels["sessions"]] == [
        {"transport": "udp", "server_port": 53}
    ]


def test_the_excluded_scenarios_run_again_on_request(tmp_path: Path) -> None:
    make_plan.build(ROOT, tmp_path, "dns", None, 42, "dpc", "dps", True, True)
    count = int((tmp_path / "rounds").read_text(encoding="utf-8"))
    ids = [
        entry["id"]
        for index in range(count)
        for entry in json.loads(
            (tmp_path / f"round-{index}" / "plan.json").read_text(encoding="utf-8")
        )["mix"]
    ]
    assert "tcp_fallback" in ids
    labels = yaml.safe_load((tmp_path / "labels.yaml").read_text(encoding="utf-8"))
    assert {"transport": "tcp", "server_port": 53} in [
        e["match"] for e in labels["sessions"]
    ]
