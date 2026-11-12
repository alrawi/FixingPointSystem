# SPDX-License-Identifier: GPL-3.0-only
"""Builds the inputs of the Gate 2 mix run from mix.yaml.

    python3 -I mix_config.py waived  --requested "hls ntp" --cargo engine/fps-agent/Cargo.toml
    python3 -I mix_config.py modules --root ROOT --work WORK [--waived "hls ntp"]
    python3 -I mix_config.py config  --role client --agent ID --out FILE [options]
    python3 -I mix_config.py labels  --out FILE [--waived "hls ntp"]
    python3 -I mix_config.py ports   [--waived "hls ntp"]
    python3 -I mix_config.py names   [--waived "hls ntp"]

``mix.yaml`` holds the entries of the byte weighted enterprise mix. A module
that the agent does not run yet can be waived. A waiver only holds while the
agent lacks the native crate of the module: once ``fps-agent`` depends on the
crate, the module is part of the mix again, so a merged agent package ends the
waiver without an edit here.
"""

from __future__ import annotations

import argparse
import importlib
import io
import re
import shutil
import subprocess
import sys
import tarfile
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
MIX_FILE = HERE / "mix.yaml"
# The pools come from the benchmarking range of RFC 2544, as in Gate 1.
CLIENT_POOL = "198.18.0.0/24"
SERVER_POOL = "198.19.0.0/24"
TOLERANCE_POINTS = 2


class MixError(Exception):
    """The mix file or an argument is not usable."""


def yaml_module() -> Any:
    """Import PyYAML, which the suite needs for its inputs."""
    try:
        return importlib.import_module("yaml")
    except ImportError as e:  # pragma: no cover - the CI installs PyYAML
        raise MixError("PyYAML is not installed") from e


def load_mix(path: Path = MIX_FILE) -> dict[str, Any]:
    """Read mix.yaml and check its shape."""
    try:
        doc = yaml_module().safe_load(path.read_text(encoding="utf-8"))
    except OSError as e:
        raise MixError(f"read {path}: {e.strerror}") from e
    if not isinstance(doc, dict) or not isinstance(doc.get("entries"), list):
        raise MixError(f"{path} has no entries list")
    seen: set[tuple[str, int]] = set()
    for entry in doc["entries"]:
        for key in (
            "module",
            "scenario",
            "weight",
            "expected_bytes",
            "params",
            "application",
            "ndpi",
            "ports",
        ):
            if key not in entry:
                raise MixError(
                    f"entry {entry.get('module')}/{entry.get('scenario')} has no {key}"
                )
        for port in entry["ports"]:
            ident = (str(port["transport"]), int(port["port"]))
            if ident in seen:
                raise MixError(f"two entries use {ident[0]} port {ident[1]}")
            seen.add(ident)
    return doc


def weights_total(doc: dict[str, Any]) -> float:
    """Return the sum of all weights, which must be 100."""
    return float(sum(float(e["weight"]) for e in doc["entries"]))


def crate_of(module: str) -> str:
    """Return the name of the native crate of a module."""
    return "fps-module-" + module.replace("_", "-")


def supported_modules(cargo: Path, modules: Iterable[str]) -> set[str]:
    """Return the modules whose native crate the agent depends on."""
    try:
        text = cargo.read_text(encoding="utf-8")
    except OSError as e:
        raise MixError(f"read {cargo}: {e.strerror}") from e
    return {
        m
        for m in modules
        if re.search(rf"^{re.escape(crate_of(m))}\s*=", text, flags=re.MULTILINE)
    }


def waived(requested: Iterable[str], supported: set[str]) -> list[str]:
    """Return the requested waivers that still hold."""
    return sorted({m for m in requested if m and m not in supported})


def included(doc: dict[str, Any], waive: Iterable[str]) -> list[dict[str, Any]]:
    """Return the entries of the modules that are not waived."""
    skip = set(waive)
    return [e for e in doc["entries"] if e["module"] not in skip]


def scenario_ref(entry: dict[str, Any]) -> str:
    """Return the `module/scenario` name of an entry."""
    return f"{entry['module']}/{entry['scenario']}"


def test_config(
    doc: dict[str, Any],
    role: str,
    agent: str,
    rate_bps: int,
    ramp_s: int,
    steady_s: int,
    ramp_down_s: int,
    seed: int,
    waive: Iterable[str],
) -> str:
    """Return the test configuration of one agent."""
    entries = included(doc, waive)
    config = {
        "apiVersion": "fixingpoint.dev/v1alpha1",
        "kind": "Test",
        "metadata": {
            "name": f"gate2_{role}",
            "description": f"The Gate 2 byte weighted enterprise mix, run by the {role} agent.",
        },
        "topology": "app_sim",
        "network": {
            "ports": [
                {"name": "client0", "location": f"{agent}/eth0"},
                {"name": "server0", "location": f"{agent}/eth0"},
            ],
            "host_pools": [
                {
                    "name": "clients",
                    "port": "client0",
                    "role": "client",
                    "ipv4": CLIENT_POOL,
                    "count": 64,
                },
                {
                    "name": "servers",
                    "port": "server0",
                    "role": "server",
                    "ipv4": SERVER_POOL,
                    "count": 8,
                },
            ],
        },
        "profile": {
            "basis": "bytes",
            "mix": [
                {
                    "scenario": scenario_ref(e),
                    "weight": e["weight"],
                    "params": e["params"],
                }
                for e in entries
            ],
        },
        "load": {
            "objective": {"type": "throughput_bps", "value": rate_bps},
            "phases": [
                {"name": "ramp_up", "duration_s": ramp_s, "shape": "linear"},
                {"name": "steady", "duration_s": steady_s, "shape": "hold"},
                {"name": "ramp_down", "duration_s": ramp_down_s, "shape": "linear"},
            ],
        },
        "timeline": {"seed": seed},
        "criteria": [{"metric": "sessions.failed_ratio", "op": "<=", "value": 0.02}],
    }
    header = (
        "# SPDX-License-Identifier: GPL-3.0-only\n"
        "#\n"
        "# Generated by tests/gates/gate2/mix_config.py from mix.yaml.\n"
    )
    return header + str(
        yaml_module().safe_dump(config, sort_keys=False, width=1_000_000)
    )


def labels(doc: dict[str, Any], waive: Iterable[str]) -> str:
    """Return the labels of the realism runner for the mix."""
    entries = included(doc, waive)
    sessions: list[dict[str, Any]] = []
    applications: dict[str, dict[str, list[str]]] = {}
    scenarios: dict[str, str] = {}
    for e in entries:
        for port in e["ports"]:
            sessions.append(
                {
                    "application": e["application"],
                    "match": {
                        "transport": port["transport"],
                        "server_port": port["port"],
                    },
                }
            )
        applications.setdefault(e["application"], {"ndpi": list(e["ndpi"])})
        if applications[e["application"]]["ndpi"] != list(e["ndpi"]):
            raise MixError(
                f"application {e['application']} lists two sets of nDPI names"
            )
        scenarios[scenario_ref(e)] = e["application"]
    aux = doc["aux"]
    applications[aux["application"]] = {"ndpi": list(aux["ndpi"])}
    out = {
        "labels": "fps-realism-labels",
        "version": 1,
        "threshold": 0.99,
        "sessions": sessions,
        "default": aux["application"],
        "applications": applications,
        "mix": {
            "scenarios": scenarios,
            "tolerance_points": TOLERANCE_POINTS,
            "basis": "bytes",
        },
    }
    header = (
        "# SPDX-License-Identifier: GPL-3.0-only\n"
        "#\n"
        "# Generated by tests/gates/gate2/mix_config.py from mix.yaml.\n"
    )
    return header + str(yaml_module().safe_dump(out, sort_keys=False, width=1_000_000))


def listen_ports(doc: dict[str, Any], waive: Iterable[str]) -> list[tuple[str, int]]:
    """Return the transport and port that the server side of each entry listens on."""
    return [
        (str(p["transport"]), int(p["port"]))
        for e in included(doc, waive)
        for p in e["ports"]
    ]


def requires_of(manifest: dict[str, Any]) -> list[str]:
    """Return the modules that a manifest requires."""
    return [
        str(r["module"]) for r in manifest.get("requires", []) if isinstance(r, dict)
    ]


def tracked_tree(root: Path, module: str, dest: Path) -> None:
    """Copy the tracked files of a module tree, so build output cannot change its digest."""
    source = root / "modules" / module
    if not source.is_dir():
        raise MixError(f"{source} is not a module directory")
    dest.mkdir(parents=True, exist_ok=True)
    listed = subprocess.run(
        [
            "git",
            "-c",
            f"safe.directory={root}",
            "-C",
            str(root),
            "ls-files",
            "-z",
            f"modules/{module}",
        ],
        capture_output=True,
        check=False,
    )
    if listed.returncode == 0 and listed.stdout:
        for rel in listed.stdout.decode().split("\0"):
            if not rel:
                continue
            target = dest / Path(rel).relative_to(f"modules/{module}")
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(root / rel, target)
        return
    shutil.copytree(
        source,
        dest,
        dirs_exist_ok=True,
        ignore=shutil.ignore_patterns("target", "__pycache__"),
    )


def set_expected_bytes(manifest_path: Path, estimates: dict[str, int]) -> None:
    """Set the `expected_bytes` of the scenarios of the mix in a staged manifest."""
    yaml = yaml_module()
    doc = yaml.safe_load(manifest_path.read_text(encoding="utf-8"))
    current = dict(doc.get("expected_bytes") or {})
    current.update(estimates)
    doc["expected_bytes"] = current
    manifest_path.write_text(
        yaml.safe_dump(doc, sort_keys=False, width=1_000_000, allow_unicode=True),
        encoding="utf-8",
    )


def stage_modules(
    root: Path, work: Path, doc: dict[str, Any], waive: Iterable[str]
) -> list[str]:
    """Stage the module trees of the mix and return them in installation order.

    A module that another module requires comes first. The staged manifests carry
    the byte estimates of mix.yaml, which most manifests do not have yet.
    """
    entries = included(doc, waive)
    estimates: dict[str, dict[str, int]] = {}
    for e in entries:
        estimates.setdefault(str(e["module"]), {})[str(e["scenario"])] = int(
            e["expected_bytes"]
        )
    order: list[str] = []

    def add(module: str) -> None:
        if module in order:
            return
        tree = work / "modules" / module
        if not tree.exists():
            tracked_tree(root, module, tree)
        manifest = tree / "manifest.yaml"
        if not manifest.is_file():
            raise MixError(f"modules/{module} has no manifest.yaml")
        for required in requires_of(
            yaml_module().safe_load(manifest.read_text(encoding="utf-8"))
        ):
            add(required)
        if module in estimates:
            set_expected_bytes(manifest, estimates[module])
        order.append(module)

    for module in estimates:
        add(module)
    for module in order:
        tree = work / "modules" / module
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
            for path in sorted(tree.rglob("*")):
                tar.add(path, arcname=str(path.relative_to(tree)), recursive=False)
        (work / f"{module}.tar.gz").write_bytes(buffer.getvalue())
    return order


def words(value: str) -> list[str]:
    """Split a list that the shell passes as one string."""
    return [w for w in re.split(r"[\s,]+", value) if w]


def main(argv: Sequence[str] | None = None) -> int:
    """Run one subcommand."""
    parser = argparse.ArgumentParser(description="Inputs of the Gate 2 mix run")
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("waived", "modules", "config", "labels", "ports", "names"):
        p = sub.add_parser(name)
        p.add_argument("--mix", type=Path, default=MIX_FILE)
        p.add_argument("--waived", default="", help="modules that the mix leaves out")
        if name == "waived":
            p.add_argument("--requested", default="")
            p.add_argument("--cargo", type=Path, required=True)
        if name == "modules":
            p.add_argument("--root", type=Path, required=True)
            p.add_argument("--work", type=Path, required=True)
        if name == "config":
            p.add_argument("--role", required=True)
            p.add_argument("--agent", required=True)
            p.add_argument("--rate-bps", type=int, default=8_000_000)
            p.add_argument("--ramp-s", type=int, default=10)
            p.add_argument("--steady-s", type=int, default=120)
            p.add_argument("--ramp-down-s", type=int, default=10)
            p.add_argument("--seed", type=int, default=7)
        if name in ("config", "labels"):
            p.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        doc = load_mix(args.mix)
        waive = words(args.waived)
        if args.command == "waived":
            modules = sorted({str(e["module"]) for e in doc["entries"]})
            print(
                " ".join(
                    waived(
                        words(args.requested), supported_modules(args.cargo, modules)
                    )
                )
            )
        elif args.command == "modules":
            args.work.mkdir(parents=True, exist_ok=True)
            print(" ".join(stage_modules(args.root, args.work, doc, waive)))
        elif args.command == "config":
            text = test_config(
                doc,
                args.role,
                args.agent,
                args.rate_bps,
                args.ramp_s,
                args.steady_s,
                args.ramp_down_s,
                args.seed,
                waive,
            )
            args.out.write_text(text, encoding="utf-8")
        elif args.command == "labels":
            args.out.write_text(labels(doc, waive), encoding="utf-8")
        elif args.command == "ports":
            for transport, port in listen_ports(doc, waive):
                print(transport, port)
        elif args.command == "names":
            print(" ".join(sorted({str(e["module"]) for e in included(doc, waive)})))
    except MixError as e:
        print(f"mix_config.py: {e}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
