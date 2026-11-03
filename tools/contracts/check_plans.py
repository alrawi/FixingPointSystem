#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Validate agent plans in the canonical JSON mapping (contract v0.1).

The checker compiles contracts/proto/fps/agent/v1/agent.proto, parses each plan
file strictly into the Plan message, confirms that the file is in canonical
form, and applies the semantic rules that the proto types cannot express.
It exits with status 1 when any finding has severity error.
"""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import re
import sys
import tempfile
from functools import lru_cache
from pathlib import Path
from typing import Any

import yaml

sys.path.insert(0, str(Path(__file__).parent))
import validate as v

PROTO_ROOT = v.CONTRACTS / "proto"
AGENT_PROTO = "fps/agent/v1/agent.proto"
PLANS = v.EXAMPLES / "plans"
ENGINE_API = "0.2.0"

DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
MAC = re.compile(r"^([0-9a-f]{2}:){5}[0-9a-f]{2}$")
METRIC = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$")

# Objectives that compile to a session rate, and those that compile to a concurrency.
RATE_OBJECTIVES = {
    "LOAD_OBJECTIVE_SESSIONS_PER_SECOND",
    "LOAD_OBJECTIVE_THROUGHPUT_BPS",
}
CONCURRENCY_OBJECTIVES = {
    "LOAD_OBJECTIVE_CONCURRENT_SESSIONS",
    "LOAD_OBJECTIVE_SIMULATED_USERS",
}


class ProtoUnavailable(RuntimeError):
    """The protobuf runtime or the proto compiler is not installed."""


def sha256_digest(data: bytes) -> str:
    """Return data's digest in the sha256:<hex> form that plans use."""
    return "sha256:" + hashlib.sha256(data).hexdigest()


def module_tree_digest(mod_dir: Path) -> str:
    """Return the module digest that contracts/README.md defines.

    Every regular file under the module directory contributes one line of the
    form `<sha256 hex>  <relative POSIX path>\\n`. Lines sort by the UTF-8 bytes
    of the path, and the digest is the SHA-256 of their concatenation.
    """
    lines = []
    for path in mod_dir.rglob("*"):
        if path.is_symlink():
            raise ValueError(
                f"module {mod_dir.name}: symbolic link {path.relative_to(mod_dir)} is not allowed"
            )
        if path.is_file():
            rel = path.relative_to(mod_dir).as_posix()
            lines.append(
                (
                    rel.encode(),
                    f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {rel}\n",
                )
            )
    lines.sort()
    return sha256_digest("".join(line for _, line in lines).encode())


@lru_cache(maxsize=1)
def plan_class() -> Any:
    """Compile the agent proto and return the generated Plan message class."""
    try:
        from google.protobuf import descriptor_pb2, descriptor_pool, message_factory
        from grpc_tools import protoc
    except ImportError as exc:
        raise ProtoUnavailable(
            f"install tools/contracts/requirements-proto.txt: {exc}"
        ) from exc
    import grpc_tools

    wkt = Path(grpc_tools.__file__).parent / "_proto"
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "agent.binpb"
        code = protoc.main(
            [
                "protoc",
                f"-I{PROTO_ROOT}",
                f"-I{wkt}",
                "--include_imports",
                f"--descriptor_set_out={out}",
                AGENT_PROTO,
            ]
        )
        if code != 0:
            raise RuntimeError(f"protoc failed on {AGENT_PROTO} with status {code}")
        fds = descriptor_pb2.FileDescriptorSet.FromString(out.read_bytes())
    pool = descriptor_pool.DescriptorPool()
    for fd in fds.file:
        pool.Add(fd)
    return message_factory.GetMessageClass(
        pool.FindMessageTypeByName("fps.agent.v1.Plan")
    )


def parse_plan(text: str) -> Any:
    """Parse plan JSON strictly. Unknown fields and wrong types raise ParseError."""
    from google.protobuf import json_format

    return json_format.Parse(text, plan_class()(), ignore_unknown_fields=False)


def canonical(plan: Any) -> dict:
    """Return the canonical proto3 JSON mapping of plan as a dict."""
    from google.protobuf import json_format

    return json_format.MessageToDict(plan)


def _usable_hosts(
    prefix: str, gateway: str, report: v.Report, file: Path, where: str
) -> int | None:
    try:
        net = ipaddress.ip_network(prefix, strict=True)
    except ValueError as exc:
        report.error(file, where, str(exc))
        return None
    usable = (
        net.num_addresses - 2
        if net.version == 4 and net.prefixlen < 31
        else net.num_addresses - 1
    )
    if gateway:
        try:
            gw = ipaddress.ip_address(gateway)
        except ValueError as exc:
            report.error(file, where, str(exc))
            return None
        if gw.version == net.version:
            if gw not in net:
                report.error(file, where, f"gateway {gw} is outside {net}")
            else:
                usable -= 1
    return usable


def check_plan(
    doc: dict, plan: Any, file: Path, report: v.Report, modules_dir: Path | None
) -> None:
    """Apply the semantic rules of contract v0.1 to one parsed plan."""
    d = canonical(plan)
    for key in ("planId", "runId", "agentId", "engineApi"):
        if not d.get(key):
            report.error(file, f"/{key}", f"{key} is required")
    if d.get("engineApi") and not SEMVER.match(d["engineApi"]):
        report.error(file, "/engineApi", "engineApi must be a version such as 0.2.0")
    elif (
        d.get("engineApi")
        and d["engineApi"].rsplit(".", 1)[0] != ENGINE_API.rsplit(".", 1)[0]
    ):
        report.warning(
            file,
            "/engineApi",
            f"engineApi {d['engineApi']} differs from contract {ENGINE_API}",
        )
    for key in ("topology", "backend"):
        if key not in d:
            report.error(file, f"/{key}", f"{key} is required")
    if "seed" not in doc:
        report.error(file, "/seed", "seed is required so that the run is repeatable")

    modules: dict[str, dict] = {}
    for i, mod in enumerate(d.get("modules", [])):
        where = f"/modules/{i}"
        name = mod.get("name", "")
        if name in modules:
            report.error(file, where + "/name", f"duplicate module '{name}'")
        modules[name] = mod
        if not DIGEST.match(mod.get("digest", "")):
            report.error(
                file, where + "/digest", "digest must be sha256:<64 lowercase hex>"
            )
        if modules_dir is not None and (modules_dir / name).is_dir():
            manifest = v.load_yaml(modules_dir / name / "manifest.yaml")
            if manifest.get("version") != mod.get("version"):
                report.error(
                    file,
                    where + "/version",
                    f"module {name} has version {manifest.get('version')}, not {mod.get('version')}",
                )
            actual = module_tree_digest(modules_dir / name)
            if actual != mod.get("digest"):
                report.error(
                    file, where + "/digest", f"module {name} has digest {actual}"
                )

    scenarios: set[str] = set()
    for i, sc in enumerate(d.get("scenarios", [])):
        where = f"/scenarios/{i}"
        ref, module, source = (
            sc.get("ref", ""),
            sc.get("module", ""),
            sc.get("source", ""),
        )
        if ref in scenarios:
            report.error(file, where + "/ref", f"duplicate scenario '{ref}'")
        scenarios.add(ref)
        if "/" not in ref or ref.split("/", 1)[0] != module:
            report.error(
                file, where + "/ref", "ref must be <module>/<scenario> and match module"
            )
        if module not in modules:
            report.error(
                file, where + "/module", f"module '{module}' is not in modules"
            )
        if sc.get("digest") != sha256_digest(source.encode()):
            report.error(file, where + "/digest", "digest does not match the source")
        try:
            scenario_doc = yaml.safe_load(source)
        except yaml.YAMLError as exc:
            report.error(file, where + "/source", f"source is not YAML: {exc}")
            continue
        if not isinstance(scenario_doc, dict):
            report.error(file, where + "/source", "source must be a YAML mapping")
            continue
        if scenario_doc.get("scenario") != ref.split("/", 1)[-1]:
            report.error(
                file,
                where + "/source",
                f"source defines scenario '{scenario_doc.get('scenario')}'",
            )
        sub = v.Report()
        if v.schema_errors(
            v.load_schema("scenario.schema.json"), scenario_doc, sub, file
        ):
            v.check_scenario(scenario_doc, file, sub)
        for finding in sub.findings:
            report.add(
                finding.severity,
                file,
                where + "/source" + finding.path,
                finding.message,
            )

    pools: dict[str, dict] = {}
    for i, pool in enumerate(d.get("hostPools", [])):
        where = f"/hostPools/{i}"
        name = pool.get("name", "")
        if name in pools:
            report.error(file, where + "/name", f"duplicate host pool '{name}'")
        pools[name] = pool
        if "role" not in pool:
            report.error(file, where + "/role", "role is required")
        offset, count, total = (
            pool.get("offset", 0),
            pool.get("count", 0),
            pool.get("poolCount", 0),
        )
        if count == 0:
            report.error(file, where + "/count", "count must be at least 1")
        if offset + count > total:
            report.error(
                file, where, f"slice {offset}+{count} exceeds poolCount {total}"
            )
        prefixes = [k for k in ("ipv4Prefix", "ipv6Prefix") if k in pool]
        if not prefixes:
            report.error(file, where, "a host pool needs ipv4Prefix or ipv6Prefix")
        for key in prefixes:
            usable = _usable_hosts(
                pool[key], pool.get("gateway", ""), report, file, where + "/" + key
            )
            if usable is not None and total > usable:
                report.error(
                    file,
                    where + "/poolCount",
                    f"poolCount {total} exceeds {usable} usable addresses",
                )
        if "macBase" in pool and not MAC.match(pool["macBase"]):
            report.error(
                file, where + "/macBase", "macBase must be lowercase colon notation"
            )

    ports = [p.get("name", "") for p in d.get("ports", [])]
    for i, name in enumerate(ports):
        if ports.index(name) != i:
            report.error(file, f"/ports/{i}/name", f"duplicate port '{name}'")
    for i, pool in enumerate(d.get("hostPools", [])):
        if pool.get("port") not in ports:
            report.error(
                file, f"/hostPools/{i}/port", f"unknown port '{pool.get('port')}'"
            )

    roles = {p.get("role") for p in pools.values()}
    topology = d.get("topology")
    if (
        topology == "TOPOLOGY_APP_SIM"
        and not {"HOST_ROLE_CLIENT", "HOST_ROLE_SERVER"} <= roles
    ):
        report.error(
            file,
            "/hostPools",
            "app_sim needs a client pool slice and a server pool slice",
        )
    if topology == "TOPOLOGY_CLIENT_SIM" and not d.get("externalServers"):
        report.error(file, "/externalServers", "client_sim needs externalServers")

    mix: dict[str, dict] = {}
    for i, entry in enumerate(d.get("mix", [])):
        where = f"/mix/{i}"
        if entry.get("id", "") in mix:
            report.error(
                file, where + "/id", f"duplicate mix entry '{entry.get('id')}'"
            )
        mix[entry.get("id", "")] = entry
        if entry.get("scenario") not in scenarios:
            report.error(
                file,
                where + "/scenario",
                f"scenario '{entry.get('scenario')}' is not in scenarios",
            )
        weight = entry.get("weight", 0)
        if not 0 < weight <= 1:
            report.error(file, where + "/weight", "weight must lie in (0, 1]")
        for key, role in (
            ("clientPool", "HOST_ROLE_CLIENT"),
            ("serverPool", "HOST_ROLE_SERVER"),
        ):
            if key in entry:
                pool = pools.get(entry[key])
                if pool is None:
                    report.error(
                        file, where + f"/{key}", f"unknown host pool '{entry[key]}'"
                    )
                elif pool.get("role") != role:
                    report.error(
                        file,
                        where + f"/{key}",
                        f"host pool '{entry[key]}' has role {pool.get('role')}",
                    )
    if mix and abs(sum(e.get("weight", 0) for e in mix.values()) - 1) > 1e-9:
        report.error(file, "/mix", "mix weights must sum to 1")

    objective = d.get("load", {}).get("objective")
    if objective is None:
        report.error(file, "/load/objective", "objective is required")
    owned: dict[str, list[tuple[int, int, int]]] = {}
    worker_ids: set[int] = set()
    for i, worker in enumerate(d.get("workers", [])):
        where = f"/workers/{i}"
        wid = worker.get("workerId", 0)
        if wid in worker_ids:
            report.error(file, where + "/workerId", f"duplicate worker {wid}")
        worker_ids.add(wid)
        for j, rng in enumerate(worker.get("hosts", [])):
            pool = pools.get(rng.get("pool"))
            if pool is None:
                report.error(
                    file,
                    where + f"/hosts/{j}/pool",
                    f"unknown host pool '{rng.get('pool')}'",
                )
                continue
            lo, n = rng.get("offset", 0), rng.get("count", 0)
            if (
                n == 0
                or lo < pool.get("offset", 0)
                or lo + n > pool.get("offset", 0) + pool.get("count", 0)
            ):
                report.error(
                    file,
                    where + f"/hosts/{j}",
                    "host range lies outside the slice of this agent",
                )
            for other_lo, other_n, other in owned.setdefault(rng["pool"], []):
                if lo < other_lo + other_n and other_lo < lo + n:
                    report.error(
                        file,
                        where + f"/hosts/{j}",
                        f"host range overlaps worker {other}",
                    )
            owned[rng["pool"]].append((lo, n, wid))
        seen: set[str] = set()
        for j, rate in enumerate(worker.get("rates", [])):
            rw = where + f"/rates/{j}"
            name = rate.get("mixEntry", "")
            if name not in mix:
                report.error(file, rw + "/mixEntry", f"unknown mix entry '{name}'")
            if name in seen:
                report.error(file, rw + "/mixEntry", f"duplicate rate for '{name}'")
            seen.add(name)
            if objective in RATE_OBJECTIVES and "sessionsPerSecond" not in rate:
                report.error(file, rw, f"{objective} needs sessionsPerSecond")
            if objective in CONCURRENCY_OBJECTIVES and "concurrentSessions" not in rate:
                report.error(file, rw, f"{objective} needs concurrentSessions")
            if rate.get("sessionsPerSecond", 1) <= 0:
                report.error(
                    file,
                    rw + "/sessionsPerSecond",
                    "sessionsPerSecond must be positive",
                )
    if not worker_ids:
        report.error(file, "/workers", "a plan needs at least one worker")

    phases = d.get("load", {}).get("phases", [])
    if not phases:
        report.error(file, "/load/phases", "a plan needs at least one load phase")
    names: set[str] = set()
    for i, phase in enumerate(phases):
        where = f"/load/phases/{i}"
        if phase.get("name", "") in names:
            report.error(
                file, where + "/name", f"duplicate phase '{phase.get('name')}'"
            )
        names.add(phase.get("name", ""))
        if plan.load.phases[i].duration.ToNanoseconds() <= 0:
            report.error(file, where + "/duration", "duration must be positive")
        if "shape" not in phase:
            report.error(file, where + "/shape", "shape is required")
        for key in ("startFraction", "endFraction"):
            if not 0 <= phase.get(key, 0) <= 1:
                report.error(file, where + f"/{key}", f"{key} must lie in [0, 1]")
        if phase.get("shape") == "PHASE_SHAPE_HOLD" and phase.get(
            "startFraction", 0
        ) != phase.get("endFraction", 0):
            report.error(
                file, where, "a hold phase needs equal start and end fractions"
            )
        if phase.get("shape") == "PHASE_SHAPE_STEP" and phase.get("steps", 0) < 1:
            report.error(
                file, where + "/steps", "a step phase needs steps of at least 1"
            )

    for i, crit in enumerate(d.get("criteria", [])):
        if not METRIC.match(crit.get("metric", "")):
            report.error(
                file, f"/criteria/{i}/metric", "metric must be a dotted catalog name"
            )
        if "op" not in crit:
            report.error(file, f"/criteria/{i}/op", "op is required")


def check_plan_file(path: Path, report: v.Report, modules_dir: Path | None) -> None:
    """Parse one plan file, confirm canonical form, and run the semantic rules."""
    from google.protobuf import json_format

    text = path.read_text(encoding="utf-8")
    try:
        doc = json.loads(text)
    except json.JSONDecodeError as exc:
        report.error(path, "", f"not JSON: {exc}")
        return
    try:
        plan = parse_plan(text)
    except json_format.ParseError as exc:
        report.error(path, "", f"does not match fps.agent.v1.Plan: {exc}")
        return
    if canonical(plan) != doc:
        report.error(
            path,
            "",
            "not in canonical form; write the proto3 JSON mapping with lowerCamelCase "
            "names and without default values",
        )
    check_plan(doc, plan, path, report, modules_dir)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "plans",
        type=Path,
        nargs="*",
        help="plan files (default: contracts/examples/plans/*.json)",
    )
    parser.add_argument(
        "--modules",
        type=Path,
        default=v.EXAMPLES / "modules",
        help="directory whose modules must match the plan digests",
    )
    parser.add_argument(
        "--print-digests",
        action="store_true",
        help="print the digest of every module under --modules and exit",
    )
    args = parser.parse_args(argv)
    if args.print_digests:
        for mod_dir in sorted(p for p in args.modules.iterdir() if p.is_dir()):
            print(f"{mod_dir.name} {module_tree_digest(mod_dir)}")
        return 0
    plans = args.plans or sorted(PLANS.glob("*.json"))
    report = v.Report()
    try:
        plan_class()
    except ProtoUnavailable as exc:
        print(f"ERROR   {exc}")
        return 1
    if not plans:
        report.error(PLANS, "", "no plan files found")
    for path in plans:
        check_plan_file(path, report, args.modules)
    for finding in report.findings:
        print(finding)
    n_err = len(report.errors)
    print(
        f"plans: {n_err} error(s), {len(report.findings) - n_err} warning(s) in {len(plans)} file(s)"
    )
    return 1 if n_err else 0


if __name__ == "__main__":
    sys.exit(main())
