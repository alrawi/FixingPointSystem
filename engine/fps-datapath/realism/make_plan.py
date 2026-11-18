#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Builds the plan and the module tree of the http1 realism run on AF_XDP (TRN-12).

The kernel run of the module (modules/http1/fixtures/capture.sh) walks the ten
scenarios one after another with a fixture peer over kernel sockets. The AF_XDP
run gives the same scenarios to the agent as ten mix entries of one plan, with
the backend BACKEND_AF_XDP, so the agent runs the sessions on the user space
datapath. Two mix entries cannot listen on one port of one server host, so each
plain scenario gets a port of its own (8080 to 8085). The https scenarios keep
the ports 8443 to 8446 that their manifests name, so the labels of the kernel
run (modules/http1/fixtures/expected.yaml) still tell them apart.

    make_plan.py REPO_ROOT OUT_DIR [--sessions N] [--seed S] [--client IF] [--server IF]
                [--keys keylog|none]

writes OUT_DIR/plan.json, the module tree OUT_DIR/modules/http1 that the plan
digests, and OUT_DIR/labels.yaml, the labels of the run.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path
from typing import Any

import yaml

# The scenario variables that the kernel run sets, see capture.sh.
VARS = {
    "body_size": 1200,
    "page_size": 3000,
    "chunk_size": 1024,
    "objects": 3,
    "object_size": 900,
    "upload_size": 2500,
}
FIRST_PLAIN_PORT = 8080
# Files of the module tree that the agent needs. The native crate, the tests,
# and the fixtures stay out, so a build cannot change the digest.
TREE = ("manifest.yaml", "scenarios")


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


def stage(root: Path, out: Path) -> Path:
    """Copies the files of the http1 module that the agent loads."""
    source = root / "modules" / "http1"
    tree = out / "modules" / "http1"
    if tree.exists():
        shutil.rmtree(tree)
    tree.mkdir(parents=True)
    for name in TREE:
        src = source / name
        if src.is_dir():
            shutil.copytree(src, tree / name)
        else:
            shutil.copy2(src, tree / name)
    return tree


def build(
    root: Path,
    out: Path,
    sessions: int,
    seed: int,
    client_if: str,
    server_if: str,
    keys: bool,
) -> None:
    tree = stage(root, out)
    manifest = yaml.safe_load((tree / "manifest.yaml").read_text(encoding="utf-8"))
    scenarios: list[dict[str, Any]] = []
    mix: list[dict[str, Any]] = []
    rates: list[dict[str, Any]] = []
    plain_ports: list[int] = []
    plain = FIRST_PLAIN_PORT
    for rel in manifest["scenarios"]:
        text = (tree / rel).read_text(encoding="utf-8")
        doc = yaml.safe_load(text)
        name = doc["scenario"]
        declared = doc.get("vars", {})
        params = {k: v for k, v in VARS.items() if k in declared}
        if name.startswith("https_"):
            port = int(declared["server_port"])
        else:
            port = plain
            plain += 1
            params["server_port"] = port
        ref = f"http1/{name}"
        scenarios.append(
            {
                "ref": ref,
                "module": "http1",
                "source": text,
                "digest": "sha256:" + hashlib.sha256(text.encode()).hexdigest(),
            }
        )
        mix.append(
            {
                "id": name,
                "scenario": ref,
                "weight": 1.0,
                "params": params,
                "clientPool": "clients",
                "serverPool": "servers",
            }
        )
        rates.append({"mixEntry": name, "sessionsPerSecond": 5})
        if not name.startswith("https_"):
            plain_ports.append(port)
    steady = max(1, sessions // 5)
    plan = {
        "planId": "run-http1-afxdp/agent-a",
        "runId": "run-http1-afxdp",
        "agentId": "agent-a",
        "testName": "http1_afxdp_realism",
        "engineApi": "0.2.0",
        "seed": str(seed),
        "topology": "TOPOLOGY_APP_SIM",
        "backend": "BACKEND_AF_XDP",
        "modules": [
            {
                "name": "http1",
                "version": manifest["version"],
                "digest": tree_digest(tree),
            }
        ],
        "scenarios": scenarios,
        "mix": mix,
        "ports": [
            {"name": "client0", "interfaceName": client_if},
            {"name": "server0", "interfaceName": server_if},
        ],
        "hostPools": [
            {
                "name": "clients",
                "port": "client0",
                "role": "HOST_ROLE_CLIENT",
                "ipv4Prefix": "10.1.0.0/24",
                "poolCount": 8,
                "count": 8,
                "macBase": "02:00:10:00:00:01",
                "osProfile": "windows11",
            },
            {
                "name": "servers",
                "port": "server0",
                "role": "HOST_ROLE_SERVER",
                "ipv4Prefix": "10.2.0.0/24",
                "poolCount": 1,
                "count": 1,
                "macBase": "02:00:30:00:00:01",
                "osProfile": "linux6",
            },
        ],
        "workers": [
            {
                "core": 0,
                "hosts": [
                    {"pool": "clients", "count": 8},
                    {"pool": "servers", "count": 1},
                ],
                "rates": rates,
            }
        ],
        "load": {
            "objective": "LOAD_OBJECTIVE_SESSIONS_PER_SECOND",
            "objectiveValue": 5 * len(mix),
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
            {"metric": "sessions.failed_ratio", "op": "CRITERION_OP_LT", "value": 0.001}
        ],
    }
    (out / "plan.json").write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")

    labels = yaml.safe_load(
        (root / "modules/http1/fixtures/expected.yaml").read_text(encoding="utf-8")
    )
    labels["sessions"] = [
        {"application": "http1", "match": {"transport": "tcp", "server_port": p}}
        for p in plain_ports
    ] + [s for s in labels["sessions"] if s["application"] != "http1"]
    if not keys:
        # No key log: the capture cannot be decrypted. The https sessions keep
        # their application and nDPI class, and the encryption check (decryption
        # and JA4) leaves the labels.
        del labels["tls"]
        for entry in labels["sessions"]:
            entry.pop("tls_profile", None)
    header = "# SPDX-License-Identifier: GPL-3.0-only\n# Generated by make_plan.py from modules/http1/fixtures/expected.yaml.\n"
    (out / "labels.yaml").write_text(
        header + yaml.safe_dump(labels, sort_keys=False, width=1_000_000),
        encoding="utf-8",
    )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument(
        "--sessions", type=int, default=20, help="about this many sessions per scenario"
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
    build(
        args.root.resolve(),
        args.out,
        args.sessions,
        args.seed,
        args.client,
        args.server,
        args.keys == "keylog",
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
