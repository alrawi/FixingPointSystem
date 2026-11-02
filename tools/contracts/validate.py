#!/usr/bin/env python3
"""Validate FixingPointSystem contracts and examples (contract v0.1).

The validator runs JSON Schema checks and the semantic rules that JSON Schema
cannot express. It exits with status 1 when any finding has severity error.
"""
from __future__ import annotations

import argparse
import ipaddress
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

import yaml
from jsonschema import Draft202012Validator

REPO = Path(__file__).resolve().parents[2]
CONTRACTS = REPO / "contracts"
SCHEMAS = CONTRACTS / "schemas"
OPENAPI = CONTRACTS / "openapi" / "fps-api-v1.yaml"
EXAMPLES = CONTRACTS / "examples"
METRICS = CONTRACTS / "metrics"
CATALOG = METRICS / "catalog.yaml"
CATALOG_SCHEMA = METRICS / "catalog.schema.json"

CORE_FUNCTIONS = {"fill": 1, "hex": 1, "rand_int": 2}
SESSION_FIELDS = {"id", "flow"}
RESERVED_ACTION_TYPES = {"raw": {"strike", "fuzzer"}}

# Statistics that a metric reference may append to a histogram name, as in
# sessions.response_time.p99. A histogram reference without one is an error.
HISTOGRAM_STATS = ("p50", "p90", "p99", "p999", "min", "max", "mean", "count")

# OpenAPI extension keyword that names the catalog metric behind a field.
METRIC_KEYWORD = "x-fps-metric"

# Schemas whose every property must name a catalog metric.
METRIC_SCHEMAS = ("RunSummary",)

# Endpoints that the specification's API table requires (method, path).
REQUIRED_OPERATIONS = {
    ("get", "/capabilities"),
    ("get", "/modules"),
    ("post", "/modules"),
    ("get", "/modules/{name}/{version}"),
    ("delete", "/modules/{name}/{version}"),
    ("post", "/configs"),
    ("get", "/configs/{id}"),
    ("post", "/configs/{id}:validate"),
    ("post", "/runs"),
    ("get", "/runs/{id}"),
    ("post", "/runs/{id}:stop"),
    ("get", "/runs/{id}/metrics"),
    ("get", "/runs/{id}/metrics:stream"),
    ("get", "/runs/{id}/report"),
    ("post", "/runs/{id}/captures"),
    ("get", "/runs/{id}/captures/{capture_id}"),
    ("get", "/agents"),
    ("post", "/agents"),
}


@dataclass
class Finding:
    severity: str
    file: str
    path: str
    message: str

    def __str__(self) -> str:
        loc = f"{self.file}{'#' + self.path if self.path else ''}"
        return f"{self.severity.upper():7} {loc}: {self.message}"


@dataclass
class Report:
    findings: list[Finding] = field(default_factory=list)

    def add(self, severity: str, file: Path | str, path: str, message: str) -> None:
        self.findings.append(Finding(severity, _rel(file), path, message))

    def error(self, file: Path | str, path: str, message: str) -> None:
        self.add("error", file, path, message)

    def warning(self, file: Path | str, path: str, message: str) -> None:
        self.add("warning", file, path, message)

    @property
    def errors(self) -> list[Finding]:
        return [f for f in self.findings if f.severity == "error"]


def _rel(p: Path | str) -> str:
    try:
        return str(Path(p).resolve().relative_to(REPO))
    except (ValueError, OSError):
        return str(p)


def load_yaml(path: Path) -> Any:
    with open(path, encoding="utf-8") as fh:
        return yaml.safe_load(fh)


def load_schema(name: str) -> dict:
    with open(SCHEMAS / name, encoding="utf-8") as fh:
        return json.load(fh)


def _pointer(parts: Iterable[Any]) -> str:
    return "/" + "/".join(str(p) for p in parts)


def schema_errors(schema: dict, doc: Any, report: Report, file: Path) -> bool:
    validator = Draft202012Validator(schema)
    ok = True
    for err in sorted(validator.iter_errors(doc), key=lambda e: list(e.absolute_path)):
        ok = False
        report.error(file, _pointer(err.absolute_path), err.message)
    return ok


# ---------------------------------------------------------------------------
# Metric catalog
# ---------------------------------------------------------------------------

METRIC_TOKEN = re.compile(r"[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)+")
FORMULA_REST = re.compile(r"^[\s0-9.+\-*/()]*$")


@dataclass
class MetricCatalog:
    """Metric definitions keyed by name, loaded from contracts/metrics/catalog.yaml."""

    metrics: dict[str, dict] = field(default_factory=dict)

    def check_reference(self, ref: str) -> str | None:
        """Return an error message when ref names no catalog metric, else None."""
        if ref in self.metrics:
            if self.metrics[ref]["type"] == "histogram":
                return (f"histogram '{ref}' needs a statistic suffix, one of "
                        f"{', '.join(HISTOGRAM_STATS)}")
            return None
        base, _, stat = ref.rpartition(".")
        if stat in HISTOGRAM_STATS and base in self.metrics:
            if self.metrics[base]["type"] == "histogram":
                return None
            return f"'{stat}' applies only to histograms, and '{base}' is a {self.metrics[base]['type']}"
        return f"unknown metric '{ref}'; add it to contracts/metrics/catalog.yaml"


def load_catalog(path: Path, report: Report) -> MetricCatalog:
    """Validate the catalog file and return its metrics. Invalid entries are left out."""
    catalog = MetricCatalog()
    doc = load_yaml(path)
    with open(CATALOG_SCHEMA, encoding="utf-8") as fh:
        schema = json.load(fh)
    if not schema_errors(schema, doc, report, path):
        return catalog
    declared = doc["labels"]
    for i, metric in enumerate(doc["metrics"]):
        where = f"/metrics/{i}"
        name = metric["name"]
        if name in catalog.metrics:
            report.error(path, where + "/name", f"duplicate metric '{name}'")
            continue
        for label in metric["labels"]:
            if label not in declared:
                report.error(path, where + "/labels", f"label '{label}' is not declared under labels")
        catalog.metrics[name] = metric
    for i, metric in enumerate(doc["metrics"]):
        where = f"/metrics/{i}"
        name = metric["name"]
        base, _, stat = name.rpartition(".")
        if stat in HISTOGRAM_STATS and catalog.metrics.get(base, {}).get("type") == "histogram":
            report.error(path, where + "/name", f"'{name}' collides with a statistic of histogram '{base}'")
        formula = metric.get("formula")
        if formula is None:
            continue
        if not FORMULA_REST.match(METRIC_TOKEN.sub("", formula)):
            report.error(path, where + "/formula", f"formula '{formula}' may only combine metric names and arithmetic")
        for ref in METRIC_TOKEN.findall(formula):
            target = catalog.metrics.get(ref)
            if ref == name:
                report.error(path, where + "/formula", f"formula of '{name}' refers to itself")
            elif target is None:
                report.error(path, where + "/formula", f"formula refers to unknown metric '{ref}'")
            elif target["type"] == "histogram":
                report.error(path, where + "/formula", f"formula refers to histogram '{ref}'")
    return catalog


_DEFAULT_CATALOG: MetricCatalog | None = None


def default_catalog() -> MetricCatalog:
    """Return the repository catalog, loaded once. Findings go to run() instead."""
    global _DEFAULT_CATALOG
    if _DEFAULT_CATALOG is None:
        _DEFAULT_CATALOG = load_catalog(CATALOG, Report())
    return _DEFAULT_CATALOG


# ---------------------------------------------------------------------------
# Template grammar
# ---------------------------------------------------------------------------

PLACEHOLDER = re.compile(r"\{\{(.*?)\}\}", re.S)
IDENT = r"[A-Za-z_][A-Za-z0-9_]*"
REFERENCE = re.compile(rf"^{IDENT}(?:\.{IDENT})*$")
CALL = re.compile(rf"^({IDENT}(?:\.{IDENT})?)\s*\((.*)\)$", re.S)
INTEGER = re.compile(r"^-?\d+$")
STRING = re.compile(r"^'[^']*'$")


@dataclass
class TemplateIssue:
    severity: str
    message: str


def _split_args(text: str) -> list[str]:
    args, depth, cur, quote = [], 0, "", False
    for ch in text:
        if ch == "'":
            quote = not quote
        if ch == "," and not quote and depth == 0:
            args.append(cur.strip())
            cur = ""
            continue
        if not quote:
            depth += ch == "("
            depth -= ch == ")"
        cur += ch
    if cur.strip():
        args.append(cur.strip())
    return args


def check_expression(expr: str, var_names: set[str]) -> list[TemplateIssue]:
    expr = expr.strip()
    issues: list[TemplateIssue] = []
    if not expr:
        return [TemplateIssue("error", "empty placeholder")]
    if INTEGER.match(expr) or STRING.match(expr):
        return issues
    call = CALL.match(expr)
    if call:
        name, raw_args = call.group(1), call.group(2)
        args = _split_args(raw_args)
        if "." in name:
            issues.append(TemplateIssue(
                "warning", f"helper '{name}' requires a module extension, which contract v0.1 does not define"))
        elif name not in CORE_FUNCTIONS:
            issues.append(TemplateIssue("error", f"unknown function '{name}'"))
        elif len(args) != CORE_FUNCTIONS[name]:
            issues.append(TemplateIssue(
                "error", f"function '{name}' takes {CORE_FUNCTIONS[name]} argument(s), got {len(args)}"))
        for arg in args:
            if INTEGER.match(arg) or STRING.match(arg):
                continue
            if REFERENCE.match(arg):
                issues.extend(check_reference(arg, var_names))
            else:
                issues.append(TemplateIssue("error", f"invalid argument '{arg}'"))
        if name == "hex" and len(args) == 1 and STRING.match(args[0]):
            value = args[0][1:-1]
            if not re.fullmatch(r"(?:[0-9a-fA-F]{2})*", value):
                issues.append(TemplateIssue("error", "hex() needs an even number of hexadecimal digits"))
        return issues
    if REFERENCE.match(expr):
        return check_reference(expr, var_names)
    return [TemplateIssue("error", f"cannot parse expression '{expr}'")]


def check_reference(ref: str, var_names: set[str]) -> list[TemplateIssue]:
    parts = ref.split(".")
    if parts[0] == "vars":
        if len(parts) != 2:
            return [TemplateIssue("error", f"'{ref}' must have the form vars.<name>")]
        if parts[1] not in var_names:
            return [TemplateIssue("error", f"undefined variable '{parts[1]}'")]
        return []
    if parts[0] == "session":
        if len(parts) != 2 or parts[1] not in SESSION_FIELDS:
            return [TemplateIssue("error", f"unknown session field in '{ref}'")]
        return []
    return [TemplateIssue("error", f"unknown reference root '{parts[0]}'")]


def check_template(text: str, var_names: set[str]) -> list[TemplateIssue]:
    issues: list[TemplateIssue] = []
    stripped = PLACEHOLDER.sub("", text)
    if "{{" in stripped or "}}" in stripped:
        issues.append(TemplateIssue("error", "unbalanced '{{' or '}}'"))
    for m in PLACEHOLDER.finditer(text):
        issues.extend(check_expression(m.group(1), var_names))
    return issues


# ---------------------------------------------------------------------------
# Scenario checks
# ---------------------------------------------------------------------------

def _walk_actions(actions: list, path: list) -> Iterable[tuple[list, str, Any]]:
    for i, action in enumerate(actions or []):
        if not isinstance(action, dict) or len(action) != 1:
            continue
        (kind, body), = action.items()
        here = path + [i, kind]
        yield here, kind, body
        if kind == "loop" and isinstance(body, dict):
            yield from _walk_actions(body.get("do", []), here + ["do"])
        if kind == "branch" and isinstance(body, list):
            for j, arm in enumerate(body):
                if isinstance(arm, dict):
                    yield from _walk_actions(arm.get("do", []), here + [j, "do"])


def _template_fields(kind: str, body: Any) -> Iterable[tuple[str, str]]:
    if kind == "send" and isinstance(body, str):
        yield "", body
    elif kind in ("connect", "accept", "expect", "loop", "delay") and isinstance(body, dict):
        for key in ("host", "port", "bytes", "min_bytes", "count", "ms"):
            if isinstance(body.get(key), str):
                yield key, body[key]
    elif kind == "set" and isinstance(body, dict):
        for key, value in body.items():
            if isinstance(value, str):
                yield key, value


def check_scenario(doc: dict, file: Path, report: Report, module: dict | None = None) -> None:
    flows = doc.get("flows", [])
    var_names = set((doc.get("vars") or {}).keys())
    # Variables bound by `set` actions are also visible to later templates.
    for flow in flows:
        for side in ("client", "server"):
            for _, kind, body in _walk_actions(flow.get(side, []), []):
                if kind == "set" and isinstance(body, dict):
                    var_names.update(body.keys())

    ids = [f.get("id") for f in flows]
    seen: set[str] = set()
    for i, fid in enumerate(ids):
        if fid in seen:
            report.error(file, f"/flows/{i}/id", f"duplicate flow id '{fid}'")
        seen.add(fid)
    by_id = {f.get("id"): f for f in flows}

    module_name = (module or {}).get("name")
    module_type = (module or {}).get("type")
    allowed_modules = {module_name} | {r.get("module") for r in (module or {}).get("requires", [])}

    deps: dict[str, list[str]] = {}
    spawned_targets: set[str] = set()
    for i, flow in enumerate(flows):
        fid = flow.get("id")
        after = flow.get("after", [])
        after = [after] if isinstance(after, str) else list(after)
        deps[fid] = after
        for dep in after:
            if dep == fid:
                report.error(file, f"/flows/{i}/after", f"flow '{fid}' depends on itself")
            elif dep not in by_id:
                report.error(file, f"/flows/{i}/after", f"unknown flow '{dep}'")
        if module is not None and flow.get("module") not in allowed_modules:
            report.error(file, f"/flows/{i}/module",
                         f"flow module '{flow.get('module')}' is neither '{module_name}' nor a required module")
        for side, opener in (("client", "connect"), ("server", "accept")):
            actions = flow.get(side)
            if actions and not (isinstance(actions[0], dict) and opener in actions[0]):
                report.error(file, f"/flows/{i}/{side}/0", f"{side} actions must start with '{opener}'")
        for side in ("client", "server"):
            for path, kind, body in _walk_actions(flow.get(side, []), ["flows", i, side]):
                if kind == "spawn" and isinstance(body, dict):
                    target = body.get("flow")
                    spawned_targets.add(target)
                    if target not in by_id:
                        report.error(file, _pointer(path), f"spawn of unknown flow '{target}'")
                    elif not by_id[target].get("spawned"):
                        report.error(file, _pointer(path), f"flow '{target}' must declare spawned: true")
                if kind == "branch" and isinstance(body, list):
                    total = sum(arm.get("p", 0) for arm in body if isinstance(arm, dict))
                    if abs(total - 1.0) > 1e-6:
                        report.error(file, _pointer(path), f"branch probabilities sum to {total:g}, not 1")
                if kind == "delay" and isinstance(body, dict) and body.get("dist") == "uniform":
                    if body.get("min_ms", 0) > body.get("max_ms", 0):
                        report.error(file, _pointer(path), "uniform delay has min_ms greater than max_ms")
                if kind in RESERVED_ACTION_TYPES and module_type not in RESERVED_ACTION_TYPES[kind]:
                    report.error(file, _pointer(path),
                                 f"'{kind}' is reserved for module types {sorted(RESERVED_ACTION_TYPES[kind])}")
                for key, text in _template_fields(kind, body):
                    for issue in check_template(text, var_names):
                        report.add(issue.severity, file, _pointer(path + ([key] if key else [])), issue.message)

    for i, flow in enumerate(flows):
        if flow.get("spawned") and flow.get("id") not in spawned_targets:
            report.warning(file, f"/flows/{i}", f"flow '{flow.get('id')}' is marked spawned but no action spawns it")

    # Cycle detection over `after`.
    state: dict[str, int] = {}

    def visit(node: str, stack: list[str]) -> None:
        state[node] = 1
        for nxt in deps.get(node, []):
            if nxt not in by_id or nxt == node:
                continue
            if state.get(nxt) == 1:
                cycle = stack[stack.index(nxt):] + [nxt] if nxt in stack else [node, nxt]
                report.error(file, "/flows", f"dependency cycle {' -> '.join(cycle)}")
            elif state.get(nxt) is None:
                visit(nxt, stack + [nxt])
        state[node] = 2

    for fid in deps:
        if state.get(fid) is None:
            visit(fid, [fid])


# ---------------------------------------------------------------------------
# Module checks
# ---------------------------------------------------------------------------

@dataclass
class ModuleIndex:
    manifests: dict[str, dict] = field(default_factory=dict)
    scenarios: dict[str, dict[str, dict]] = field(default_factory=dict)

    def resolve(self, ref: str) -> tuple[str, dict] | None | str:
        """Return (module, scenario doc), None when missing, or 'ambiguous'."""
        if "/" in ref:
            mod, name = ref.split("/", 1)
            doc = self.scenarios.get(mod, {}).get(name)
            return (mod, doc) if doc else None
        hits = [(m, s[ref]) for m, s in self.scenarios.items() if ref in s]
        if len(hits) > 1:
            return "ambiguous"
        return hits[0] if hits else None


def check_module_dir(mod_dir: Path, report: Report, index: ModuleIndex) -> None:
    manifest_path = mod_dir / "manifest.yaml"
    if not manifest_path.exists():
        report.error(mod_dir, "", "module directory has no manifest.yaml")
        return
    manifest = load_yaml(manifest_path)
    if not schema_errors(load_schema("module-manifest.schema.json"), manifest, report, manifest_path):
        return
    name = manifest["name"]
    if name != mod_dir.name:
        report.error(manifest_path, "/name", f"module name '{name}' differs from directory '{mod_dir.name}'")
    if name in index.manifests:
        report.error(manifest_path, "/name", f"duplicate module '{name}'")
    index.manifests[name] = manifest
    index.scenarios.setdefault(name, {})

    for i, req in enumerate(manifest.get("requires", [])):
        if req["module"] == name:
            report.error(manifest_path, f"/requires/{i}", "module requires itself")
    if "parameters" in manifest:
        try:
            Draft202012Validator.check_schema(manifest["parameters"])
        except Exception as exc:  # jsonschema raises SchemaError
            report.error(manifest_path, "/parameters", f"invalid parameter schema: {exc.message}")
    for key, items, attr in (("extension", [manifest.get("extension")], "wasm"),
                             ("fixtures", manifest.get("fixtures", []), "capture"),
                             ("assets", manifest.get("assets", []), "path")):
        for i, item in enumerate(items):
            if item and item.get(attr) and not (mod_dir / item[attr]).exists():
                report.error(manifest_path, f"/{key}" + (f"/{i}" if key != "extension" else "") + f"/{attr}",
                             f"file '{item[attr]}' does not exist")

    scenario_schema = load_schema("scenario.schema.json")
    for i, rel in enumerate(manifest.get("scenarios", [])):
        spath = mod_dir / rel
        if not spath.exists():
            report.error(manifest_path, f"/scenarios/{i}", f"scenario file '{rel}' does not exist")
            continue
        doc = load_yaml(spath)
        if not schema_errors(scenario_schema, doc, report, spath):
            continue
        sname = doc["scenario"]
        if sname in index.scenarios[name]:
            report.error(spath, "/scenario", f"duplicate scenario '{sname}' in module '{name}'")
        index.scenarios[name][sname] = doc
        check_scenario(doc, spath, report, manifest)


# ---------------------------------------------------------------------------
# Test configuration checks
# ---------------------------------------------------------------------------

def check_test_config(doc: dict, file: Path, report: Report, index: ModuleIndex,
                      catalog: MetricCatalog | None = None) -> None:
    if not schema_errors(load_schema("test-config.schema.json"), doc, report, file):
        return
    catalog = catalog or default_catalog()
    for i, criterion in enumerate(doc.get("criteria", [])):
        problem = catalog.check_reference(criterion["metric"])
        if problem:
            report.error(file, f"/criteria/{i}/metric", problem)
    net = doc["network"]
    port_names = [p["name"] for p in net["ports"]]
    for i, name in enumerate(port_names):
        if port_names.index(name) != i:
            report.error(file, f"/network/ports/{i}/name", f"duplicate port '{name}'")
    pools = {}
    for i, pool in enumerate(net["host_pools"]):
        where = f"/network/host_pools/{i}"
        if pool["name"] in pools:
            report.error(file, where + "/name", f"duplicate host pool '{pool['name']}'")
        pools[pool["name"]] = pool
        if pool["port"] not in port_names:
            report.error(file, where + "/port", f"unknown port '{pool['port']}'")
        for family in ("ipv4", "ipv6"):
            if family not in pool:
                continue
            try:
                net_obj = ipaddress.ip_network(pool[family], strict=False)
            except ValueError as exc:
                report.error(file, where + f"/{family}", str(exc))
                continue
            size = net_obj.num_addresses
            usable = size - 2 if family == "ipv4" and net_obj.prefixlen < 31 else size
            reserved = 0
            if pool.get("gateway"):
                try:
                    gw = ipaddress.ip_address(pool["gateway"])
                    if gw not in net_obj:
                        report.error(file, where + "/gateway", f"gateway {gw} is outside {net_obj}")
                    else:
                        reserved = 1
                except ValueError as exc:
                    report.error(file, where + "/gateway", str(exc))
            if pool["count"] > usable - reserved:
                report.error(file, where + "/count",
                             f"count {pool['count']} exceeds {usable - reserved} usable addresses in {net_obj}")

    roles = {p["role"] for p in pools.values()}
    topology = doc["topology"]
    if topology == "app_sim" and roles != {"client", "server"}:
        report.error(file, "/network/host_pools", "app_sim needs at least one client pool and one server pool")
    if topology == "client_sim":
        if "client" not in roles:
            report.error(file, "/network/host_pools", "client_sim needs a client pool")
        if not net.get("external_servers"):
            report.error(file, "/network/external_servers", "client_sim needs external_servers")
    if topology == "server_sim" and "server" not in roles:
        report.error(file, "/network/host_pools", "server_sim needs a server pool")

    for i, entry in enumerate(doc["profile"]["mix"]):
        where = f"/profile/mix/{i}"
        resolved = index.resolve(entry["scenario"])
        if resolved == "ambiguous":
            report.error(file, where + "/scenario",
                         f"scenario '{entry['scenario']}' exists in several modules; use <module>/<scenario>")
            continue
        if resolved is None:
            report.error(file, where + "/scenario", f"unknown scenario '{entry['scenario']}'")
            continue
        mod_name, _ = resolved
        params_schema = index.manifests[mod_name].get("parameters")
        if entry.get("params") and params_schema:
            for err in Draft202012Validator(params_schema).iter_errors(entry["params"]):
                report.error(file, where + "/params" + _pointer(err.absolute_path).rstrip("/"), err.message)
        for key, role in (("client_pool", "client"), ("server_pool", "server")):
            if key in entry:
                pool = pools.get(entry[key])
                if pool is None:
                    report.error(file, where + f"/{key}", f"unknown host pool '{entry[key]}'")
                elif pool["role"] != role:
                    report.error(file, where + f"/{key}", f"host pool '{entry[key]}' has role '{pool['role']}'")

    names = [p["name"] for p in doc["load"]["phases"]]
    for i, name in enumerate(names):
        if names.index(name) != i:
            report.error(file, f"/load/phases/{i}/name", f"duplicate phase '{name}'")


# ---------------------------------------------------------------------------
# OpenAPI checks
# ---------------------------------------------------------------------------

def _null_values(doc: Any, path: tuple = ()) -> Iterable[str]:
    if isinstance(doc, dict):
        for key, value in doc.items():
            if value is None and key != "default":
                yield _pointer(path + (key,))
            yield from _null_values(value, path + (key,))
    elif isinstance(doc, list):
        for i, value in enumerate(doc):
            yield from _null_values(value, path + (i,))


def _metric_annotations(doc: Any, path: tuple = ()) -> Iterable[tuple[str, Any]]:
    if isinstance(doc, dict):
        for key, value in doc.items():
            if key == METRIC_KEYWORD:
                yield _pointer(path + (key,)), value
            else:
                yield from _metric_annotations(value, path + (key,))
    elif isinstance(doc, list):
        for i, value in enumerate(doc):
            yield from _metric_annotations(value, path + (i,))


def check_openapi_metrics(doc: dict, path: Path, report: Report, catalog: MetricCatalog) -> None:
    """Check every x-fps-metric annotation and require one on each metric schema property."""
    for pointer, ref in _metric_annotations(doc):
        if not isinstance(ref, str):
            report.error(path, pointer, f"{METRIC_KEYWORD} must be a metric name string")
            continue
        problem = catalog.check_reference(ref)
        if problem:
            report.error(path, pointer, problem)
    schemas = (doc.get("components") or {}).get("schemas") or {}
    for name in METRIC_SCHEMAS:
        for prop, body in ((schemas.get(name) or {}).get("properties") or {}).items():
            if not isinstance(body, dict) or METRIC_KEYWORD not in body:
                report.error(path, f"/components/schemas/{name}/properties/{prop}",
                             f"property names no catalog metric; add {METRIC_KEYWORD}")


def check_openapi(path: Path, report: Report, catalog: MetricCatalog | None = None) -> None:
    doc = load_yaml(path)
    check_openapi_metrics(doc, path, report, catalog or default_catalog())
    try:
        from openapi_spec_validator import validate
    except ImportError:
        report.warning(path, "", "openapi-spec-validator is not installed; skipping OpenAPI structural checks")
    else:
        try:
            validate(doc, base_uri=path.resolve().as_uri())
        except Exception as exc:  # the library raises several exception types
            report.error(path, "", f"OpenAPI validation failed: {str(exc).splitlines()[0][:300]}")
    for pointer in _null_values(doc):
        report.error(path, pointer, "null value; quote strings that contain commas inside flow mappings")
    op_ids: dict[str, str] = {}
    present = set()
    for route, item in (doc.get("paths") or {}).items():
        for method, op in item.items():
            if method not in {"get", "put", "post", "delete", "patch"}:
                continue
            present.add((method, route))
            oid = op.get("operationId")
            if not oid:
                report.error(path, f"/paths/{route}/{method}", "operation has no operationId")
            elif oid in op_ids:
                report.error(path, f"/paths/{route}/{method}", f"duplicate operationId '{oid}'")
            else:
                op_ids[oid] = route
            if op.get("x-required-scope") not in {"read", "run", "admin"}:
                report.error(path, f"/paths/{route}/{method}", "x-required-scope must be read, run, or admin")
    for method, route in sorted(REQUIRED_OPERATIONS - present):
        report.error(path, "/paths", f"missing required operation {method.upper()} {route}")


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def run(modules_dirs: list[Path], tests: list[Path], openapi: Path | None,
        catalog_path: Path = CATALOG) -> Report:
    report = Report()
    for schema_path in [SCHEMAS / n for n in ("module-manifest.schema.json", "scenario.schema.json",
                                              "test-config.schema.json")] + [CATALOG_SCHEMA]:
        try:
            with open(schema_path, encoding="utf-8") as fh:
                Draft202012Validator.check_schema(json.load(fh))
        except Exception as exc:
            report.error(schema_path, "", f"invalid JSON Schema: {exc}")
    catalog = load_catalog(catalog_path, report)
    if openapi is not None:
        check_openapi(openapi, report, catalog)
    index = ModuleIndex()
    for root in modules_dirs:
        for mod_dir in sorted(p for p in root.iterdir() if p.is_dir()):
            check_module_dir(mod_dir, report, index)
    for test in tests:
        check_test_config(load_yaml(test), test, report, index, catalog)
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--modules", type=Path, action="append",
                        help="directory that holds module directories (default: contracts/examples/modules)")
    parser.add_argument("--tests", type=Path, nargs="*",
                        help="test configuration files (default: contracts/examples/tests/*.yaml)")
    parser.add_argument("--no-openapi", action="store_true", help="skip the OpenAPI document")
    parser.add_argument("--catalog", type=Path, default=CATALOG,
                        help="metric catalog (default: contracts/metrics/catalog.yaml)")
    parser.add_argument("--quiet", action="store_true", help="print only errors")
    args = parser.parse_args(argv)

    modules = args.modules or [EXAMPLES / "modules"]
    tests = args.tests if args.tests is not None else sorted((EXAMPLES / "tests").glob("*.yaml"))
    report = run(modules, tests, None if args.no_openapi else OPENAPI, args.catalog)
    for finding in report.findings:
        if not args.quiet or finding.severity == "error":
            print(finding)
    n_err, n_warn = len(report.errors), len(report.findings) - len(report.errors)
    print(f"contracts: {n_err} error(s), {n_warn} warning(s)")
    return 1 if n_err else 0


if __name__ == "__main__":
    sys.exit(main())
