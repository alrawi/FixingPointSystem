#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Builds the plans and the module tree of the AF_XDP realism run of a module (TRN-12, TRN-36).

The kernel run of a module (modules/MODULE/fixtures/capture.sh) walks the
scenarios with a fixture peer over kernel sockets. The AF_XDP run gives the same
scenarios to the agent as mix entries of plans with the backend BACKEND_AF_XDP,
so the agent runs the sessions on the user space datapath, and it scores the
capture with the labels of the kernel run (modules/MODULE/fixtures/expected.yaml).

    make_plan.py REPO_ROOT OUT_DIR MODULE [--sessions N] [--seed S] [--client IF]
                 [--server IF] [--keys keylog|none]

Every scenario of the manifest is a mix entry, except the scenarios that the
profile of the module leaves out (profiles.yaml, which holds the variables and
ports that capture.sh sets). A plan listens once for each port of a server host,
so two scenarios on one port cannot share a plan. The scenarios of a module
therefore run in rounds, and a round holds scenarios until two of them would
listen on one port. All rounds go into one capture, so the labels keep the
ports of the kernel run. A round uses its own client addresses and its own seed,
so no two sessions of the capture share a 5-tuple.

The script writes OUT_DIR/round-N/plan.json for each round, the module tree
OUT_DIR/modules/MODULE that the plans digest, OUT_DIR/labels.yaml, the labels of
the run, OUT_DIR/rounds, one line with the number of rounds, and
OUT_DIR/decrypted.args, the arguments of the decrypted check, one on each line.
It prints the number of rounds.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import shutil
import sys
from pathlib import Path
from typing import Any

import yaml

HERE = Path(__file__).resolve().parent
# The rate of one mix entry, in sessions per second, when it has sessions to spare.
RATE = 5
# Files of the module that the agent never reads. They stay out of the staged
# tree, so a build or a test cannot change the digest.
SKIP = {"native", "tests", "fixtures", "README.md", "bench", "results", "target"}
TEMPLATE_VAR = re.compile(r"^\{\{\s*vars\.(\w+)\s*\}\}$")


def tree_digest(directory: Path) -> str:
    """The tree digest of the agent: sha256 over the sorted `<sha256>  <path>` lines."""
    lines = []
    for path in sorted(p for p in directory.rglob("*") if p.is_file()):
        rel = path.relative_to(directory).as_posix()
        lines.append((rel, f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {rel}\n"))
    lines.sort(key=lambda item: item[0].encode())
    return (
        "sha256:"
        + hashlib.sha256("".join(line for _, line in lines).encode()).hexdigest()
    )


def load_profile(module: str) -> dict[str, Any]:
    profiles = yaml.safe_load((HERE / "profiles.yaml").read_text(encoding="utf-8"))
    if module not in profiles:
        raise SystemExit(
            f"make_plan.py: {module} has no profile in profiles.yaml "
            f"(modules with a profile: {', '.join(sorted(profiles))})"
        )
    profile: dict[str, Any] = profiles[module]
    return profile


def stage(root: Path, out: Path, module: str) -> Path:
    """Copies the files of the module that the agent loads."""
    source = root / "modules" / module
    tree = out / "modules" / module
    if tree.exists():
        shutil.rmtree(tree)
    tree.mkdir(parents=True)
    for item in source.iterdir():
        if item.name in SKIP:
            continue
        if item.is_dir():
            shutil.copytree(item, tree / item.name)
        else:
            shutil.copy2(item, tree / item.name)
    return tree


def entry_port(doc: dict[str, Any], params: dict[str, Any]) -> tuple[str, int]:
    """The transport and the port of the entry flow, which the server listens on."""
    flow = doc["flows"][0]
    l4 = flow.get("l4", "tcp")
    accept = next(
        (a["accept"] for a in flow.get("server", []) if "accept" in a),
        None,
    )
    if accept is None:
        raise SystemExit(f"scenario {doc['scenario']}: the entry flow has no accept")
    raw = str(accept["port"]).strip()
    match = TEMPLATE_VAR.match(raw)
    if match:
        name = match.group(1)
        value = params.get(name, doc.get("vars", {}).get(name))
        if value is None:
            raise SystemExit(f"scenario {doc['scenario']}: no value for {name}")
        return l4, int(value)
    return l4, int(raw)


def rounds_of(entries: list[dict[str, Any]]) -> list[list[dict[str, Any]]]:
    """Groups the entries so that no round listens twice on one transport and port."""
    rounds: list[list[dict[str, Any]]] = []
    for entry in entries:
        for group in rounds:
            if all(e["listen"] != entry["listen"] for e in group):
                group.append(entry)
                break
        else:
            rounds.append([entry])
    return rounds


def build_plan(
    module: str,
    version: str,
    digest: str,
    group: list[dict[str, Any]],
    index: int,
    seed: int,
    client_if: str,
    server_if: str,
    client_hosts: int,
    failed_ratio: float,
) -> dict[str, Any]:
    steady = max(1, *(math.ceil(e["sessions"] / RATE) for e in group))
    rates = [
        {"mixEntry": e["id"], "sessionsPerSecond": e["sessions"] / steady}
        for e in group
    ]
    return {
        "planId": f"run-{module}-afxdp-{index}/agent-a",
        "runId": f"run-{module}-afxdp-{index}",
        "agentId": "agent-a",
        "testName": f"{module}_afxdp_realism",
        "engineApi": "0.2.0",
        "seed": str(seed + index),
        "topology": "TOPOLOGY_APP_SIM",
        "backend": "BACKEND_AF_XDP",
        "modules": [{"name": module, "version": version, "digest": digest}],
        "scenarios": [e["scenario"] for e in group],
        "mix": [
            {
                "id": e["id"],
                "scenario": e["scenario"]["ref"],
                "weight": 1.0,
                "params": e["params"],
                "clientPool": "clients",
                "serverPool": "servers",
            }
            for e in group
        ],
        "ports": [
            {"name": "client0", "interfaceName": client_if},
            {"name": "server0", "interfaceName": server_if},
        ],
        "hostPools": [
            {
                "name": "clients",
                "port": "client0",
                "role": "HOST_ROLE_CLIENT",
                "ipv4Prefix": f"10.1.{index}.0/24",
                "poolCount": client_hosts,
                "count": client_hosts,
                "macBase": f"02:00:10:{index:02x}:00:01",
                "osProfile": "windows11",
            },
            {
                "name": "servers",
                "port": "server0",
                "role": "HOST_ROLE_SERVER",
                "ipv4Prefix": "10.2.0.0/24",
                "poolCount": 1,
                "count": 1,
                "macBase": f"02:00:30:{index:02x}:00:01",
                "osProfile": "linux6",
            },
        ],
        "workers": [
            {
                "core": 0,
                "hosts": [
                    {"pool": "clients", "count": client_hosts},
                    {"pool": "servers", "count": 1},
                ],
                "rates": rates,
            }
        ],
        "load": {
            "objective": "LOAD_OBJECTIVE_SESSIONS_PER_SECOND",
            "objectiveValue": sum(r["sessionsPerSecond"] for r in rates),
            "phases": [
                {
                    "name": "steady",
                    "duration": f"{steady}s",
                    "shape": "PHASE_SHAPE_HOLD",
                    "startFraction": 1,
                    "endFraction": 1,
                }
            ],
        },
        "warmup": "0s",
        "criteria": [
            {
                "metric": "sessions.failed_ratio",
                "op": "CRITERION_OP_LT",
                "value": failed_ratio,
            }
        ],
    }


def prune_labels(labels: dict[str, Any], profile: dict[str, Any]) -> None:
    """Takes the labels of the excluded scenarios out of the labels of the kernel run.

    The profile names them: drop_rules are the `match` fields of session rules,
    drop_applications are applications that only excluded scenarios carry, and a
    scenario of the structure section goes with the excluded scenario whose
    file it names.
    """
    rules = profile.get("drop_rules", [])
    apps = set(profile.get("drop_applications", []))
    labels["sessions"] = [
        entry
        for entry in labels["sessions"]
        if entry["match"] not in rules and entry["application"] not in apps
    ]
    for name in apps:
        labels.get("applications", {}).pop(name, None)
        for section in ("arrivals", "packets", "rungs"):
            group = labels.get("temporal", {}).get(section, {})
            group.get("applications", {}).pop(name, None)
    temporal = labels.get("temporal", {})
    for section in ("arrivals", "packets", "rungs"):
        if section in temporal and not temporal[section].get("applications"):
            del temporal[section]
    if "temporal" in labels and not any(
        isinstance(v, dict) for v in labels["temporal"].values()
    ):
        del labels["temporal"]
    excluded = set(profile.get("excluded", {}))
    structure = labels.get("structure")
    if structure:
        structure["scenarios"] = [
            entry
            for entry in structure["scenarios"]
            if Path(entry["file"]).stem not in excluded
        ]


def build(
    root: Path,
    out: Path,
    module: str,
    sessions: int | None,
    seed: int,
    client_if: str,
    server_if: str,
    keys: bool,
) -> int:
    """Writes the plans, the module tree, and the labels. Returns the number of rounds."""
    profile = load_profile(module)
    tree = stage(root, out, module)
    manifest = yaml.safe_load((tree / "manifest.yaml").read_text(encoding="utf-8"))
    digest = tree_digest(tree)
    per_scenario = sessions if sessions is not None else int(profile.get("sessions", 20))
    client_hosts = int(profile.get("client_hosts", 8))
    # The scenarios that capture.sh leaves out, and the scenarios that the AF_XDP
    # run leaves out until the agent runs them (the exclusion list of TRN-36).
    skipped = set(profile.get("skip", [])) | set(profile.get("excluded", {}))
    entries: list[dict[str, Any]] = []
    for rel in manifest["scenarios"]:
        text = (tree / rel).read_text(encoding="utf-8")
        doc = yaml.safe_load(text)
        name = doc["scenario"]
        if name in skipped:
            continue
        declared = doc.get("vars", {})
        values = {
            **profile.get("vars", {}),
            **profile.get("scenario_vars", {}).get(name, {}),
        }
        params = {k: v for k, v in values.items() if k in declared}
        if name in profile.get("ports", {}):
            params["server_port"] = profile["ports"][name]
        l4, port = entry_port(doc, params)
        ref = f"{module}/{name}"
        entries.append(
            {
                "id": name,
                "listen": (l4, port),
                "sessions": int(profile.get("scenario_sessions", {}).get(name, per_scenario)),
                "params": params,
                "declared": declared,
                "scenario": {
                    "ref": ref,
                    "module": module,
                    "source": text,
                    "digest": "sha256:" + hashlib.sha256(text.encode()).hexdigest(),
                },
            }
        )
    groups = rounds_of(entries)
    for index, group in enumerate(groups):
        # The addresses that the scenarios announce in their payload are the
        # first hosts of the pools of the round.
        for e in group:
            if "server_ip" in e["declared"] and "server_ip" not in e["params"]:
                e["params"]["server_ip"] = "10.2.0.1"
            if "client_ip" in e["declared"] and "client_ip" not in e["params"]:
                e["params"]["client_ip"] = f"10.1.{index}.1"
        plan = build_plan(
            module,
            manifest["version"],
            digest,
            group,
            index,
            seed,
            client_if,
            server_if,
            client_hosts,
            float(profile.get("max_failed_ratio", 0.001)),
        )
        directory = out / f"round-{index}"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "plan.json").write_text(
            json.dumps(plan, indent=2) + "\n", encoding="utf-8"
        )
    (out / "rounds").write_text(f"{len(groups)}\n", encoding="utf-8")
    # The arguments of `realism decrypted`, one on each line, when the kernel run
    # of the module scores a decrypted view. The script adds the capture, the
    # session count, and the parsers.
    decrypted = profile.get("decrypted")
    lines: list[str] = []
    if keys and decrypted:
        lines = ["--labels", str(decrypted["labels"])]
        for key, flag in (
            ("starttls_port", "--starttls-port"),
            ("view_port", "--view-port"),
            ("plaintext", "--plaintext"),
        ):
            if key in decrypted:
                lines += [flag, str(decrypted[key])]
    (out / "decrypted.args").write_text(
        "".join(f"{line}\n" for line in lines), encoding="utf-8"
    )

    labels = yaml.safe_load(
        (root / "modules" / module / "fixtures" / "expected.yaml").read_text(
            encoding="utf-8"
        )
    )
    prune_labels(labels, profile)
    # The structure section names scenario files relative to the labels file.
    for entry in labels.get("structure", {}).get("scenarios", []):
        entry["file"] = str(
            (root / "modules" / module / "fixtures" / entry["file"]).resolve()
        )
    if not keys:
        # No key log: the capture cannot be decrypted. The https sessions keep
        # their application and nDPI class, and the encryption check (decryption
        # and JA4) leaves the labels.
        labels.pop("tls", None)
        for entry in labels["sessions"]:
            entry.pop("tls_profile", None)
    header = (
        "# SPDX-License-Identifier: GPL-3.0-only\n"
        f"# Generated by make_plan.py from modules/{module}/fixtures/expected.yaml.\n"
    )
    (out / "labels.yaml").write_text(
        header + yaml.safe_dump(labels, sort_keys=False, width=1_000_000),
        encoding="utf-8",
    )
    return len(groups)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("module")
    parser.add_argument(
        "--sessions",
        type=int,
        help="about this many sessions per scenario, default the profile of the module",
    )
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--client", default="dpc")
    parser.add_argument("--server", default="dps")
    parser.add_argument(
        "--keys",
        choices=("keylog", "none"),
        default="keylog",
        help="keylog: the capture carries the TLS secrets and the labels ask for decryption",
    )
    args = parser.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    rounds = build(
        args.root.resolve(),
        args.out,
        args.module,
        args.sessions,
        args.seed,
        args.client,
        args.server,
        args.keys == "keylog",
    )
    print(rounds)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
