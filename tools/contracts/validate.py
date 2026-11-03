#!/usr/bin/env python3
"""Validate FixingPointSystem contracts and examples (scenario contract v0.2).

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
# Template grammar (scenario contract v0.2)
# ---------------------------------------------------------------------------

PLACEHOLDER = re.compile(r"\{\{(.*?)\}\}", re.S)
IDENT_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
NUMBER_RE = re.compile(r"-?\d+(?:\.\d+)?")
HEX_RE = re.compile(r"(?:[0-9a-fA-F]{2})*")

# Value types of template values and function parameters. They match the
# value-type enum of contracts/extension/fps-extension.wit.
VALUE_TYPES = ("int", "float", "boolean", "text", "bytes", "any")

# Template roots that are not module names. A module may not take one of
# these names, because a template could not tell the two apart.
RESERVED_ROOTS = {"vars", "session", "last", "true", "false"}

# Calls may nest at most this deep, counting the outermost call as 1.
MAX_CALL_DEPTH = 8

SESSION_FIELDS = {"id": "int", "flow": "text"}


@dataclass(frozen=True)
class Param:
    name: str
    type: str
    required: bool = True


@dataclass(frozen=True)
class FunctionSig:
    """Signature of one template function. Every call renders bytes, except
    that core functions declare their own result type."""

    name: str
    params: tuple[Param, ...]
    result: str = "bytes"


CORE_FUNCTIONS: dict[str, FunctionSig] = {
    "fill": FunctionSig("fill", (Param("n", "int"),), "text"),
    "hex": FunctionSig("hex", (Param("s", "text"),), "bytes"),
    "rand_int": FunctionSig("rand_int", (Param("a", "int"), Param("b", "int")), "int"),
}

# Upper bound of the UDP payload over IPv4, and the default datagram limit of
# an expect on a UDP flow.
MAX_DATAGRAM = 65507


@dataclass
class TemplateIssue:
    severity: str
    message: str


class TemplateSyntaxError(ValueError):
    """A placeholder that the v0.2 grammar cannot parse."""


@dataclass
class Lit:
    type: str
    value: Any


@dataclass
class Ref:
    parts: list[str]


@dataclass
class Call:
    module: str | None
    name: str
    positional: list[Any]
    keywords: list[tuple[str, Any]]

    @property
    def qualified(self) -> str:
        return f"{self.module}.{self.name}" if self.module else self.name


@dataclass
class TemplateScope:
    """What a template may refer to at one point of a flow program.

    vars maps every visible variable to its value type, or to 'any' when only
    a run time value fixes the type. modules maps each callable module to its
    function catalog, or to None when its manifest is not available. A scope
    whose modules is None leaves module calls unchecked. last is True when an
    expect earlier on the same side always runs before this point.
    """

    vars: dict[str, str] = field(default_factory=dict)
    modules: dict[str, dict[str, FunctionSig] | None] | None = None
    module_name: str | None = None
    last: bool = True


def _tokenize(text: str) -> list[tuple[str, str]]:
    tokens: list[tuple[str, str]] = []
    i = 0
    while i < len(text):
        ch = text[i]
        if ch.isspace():
            i += 1
        elif ch in "().,=":
            tokens.append((ch, ch))
            i += 1
        elif ch == "'":
            end = text.find("'", i + 1)
            if end < 0:
                raise TemplateSyntaxError("unterminated string literal")
            tokens.append(("str", text[i + 1:end]))
            i = end + 1
        elif (m := NUMBER_RE.match(text, i)) and (ch != "-" or m.end() > i + 1):
            kind = "float" if "." in m.group() else "int"
            tokens.append((kind, m.group()))
            i = m.end()
        elif m := IDENT_RE.match(text, i):
            tokens.append(("ident", m.group()))
            i = m.end()
        else:
            raise TemplateSyntaxError(f"unexpected character '{ch}'")
    return tokens


class _Parser:
    def __init__(self, text: str):
        self.tokens = _tokenize(text)
        self.pos = 0

    def peek(self, offset: int = 0) -> str | None:
        j = self.pos + offset
        return self.tokens[j][0] if j < len(self.tokens) else None

    def take(self, kind: str) -> str:
        if self.peek() != kind:
            got = self.tokens[self.pos][1] if self.pos < len(self.tokens) else "end of expression"
            raise TemplateSyntaxError(f"expected '{kind}', got '{got}'")
        value = self.tokens[self.pos][1]
        self.pos += 1
        return value

    def parse(self) -> Any:
        node = self.expression(1)
        if self.peek() is not None:
            raise TemplateSyntaxError(f"unexpected '{self.tokens[self.pos][1]}'")
        return node

    def expression(self, depth: int) -> Any:
        kind = self.peek()
        if kind in ("int", "float"):
            text = self.take(kind)
            return Lit(kind, int(text) if kind == "int" else float(text))
        if kind == "str":
            return Lit("text", self.take("str"))
        if kind != "ident":
            raise TemplateSyntaxError("expected a literal, a reference, or a call")
        first = self.take("ident")
        if first in ("true", "false") and self.peek() not in (".", "("):
            return Lit("boolean", first == "true")
        parts = [first]
        while self.peek() == ".":
            self.take(".")
            parts.append(self.take("ident"))
        if self.peek() != "(":
            return Ref(parts)
        if len(parts) > 2:
            raise TemplateSyntaxError(f"'{'.'.join(parts)}' is not a function name; use <module>.<function>")
        if depth > MAX_CALL_DEPTH:
            raise TemplateSyntaxError(f"calls nest deeper than {MAX_CALL_DEPTH} levels")
        self.take("(")
        call = Call(parts[0] if len(parts) == 2 else None, parts[-1], [], [])
        if self.peek() != ")":
            while True:
                if self.peek() == "ident" and self.peek(1) == "=":
                    name = self.take("ident")
                    self.take("=")
                    call.keywords.append((name, self.expression(depth + 1)))
                elif call.keywords:
                    raise TemplateSyntaxError("a positional argument follows a keyword argument")
                else:
                    call.positional.append(self.expression(depth + 1))
                if self.peek() != ",":
                    break
                self.take(",")
        self.take(")")
        return call


def parse_expression(text: str) -> Any:
    """Parse one placeholder expression into Lit, Ref, and Call nodes."""
    return _Parser(text).parse()


def compatible(param_type: str, arg_type: str) -> bool:
    """Whether a value of arg_type may bind a parameter of param_type."""
    if "any" in (param_type, arg_type) or param_type == arg_type:
        return True
    return (param_type, arg_type) in {("float", "int"), ("bytes", "text")}


def value_type(value: Any) -> str:
    """Type of a scenario variable value as YAML loads it."""
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "int"
    if isinstance(value, float):
        return "float"
    return "text"


def _check_reference(ref: Ref, scope: TemplateScope, issues: list[TemplateIssue]) -> str:
    parts, text = ref.parts, ".".join(ref.parts)
    root = parts[0]
    if root == "vars":
        if len(parts) != 2:
            issues.append(TemplateIssue("error", f"'{text}' must have the form vars.<name>"))
            return "any"
        if parts[1] not in scope.vars:
            issues.append(TemplateIssue("error", f"undefined variable '{parts[1]}'"))
            return "any"
        return scope.vars[parts[1]]
    if root == "session":
        if len(parts) != 2 or parts[1] not in SESSION_FIELDS:
            issues.append(TemplateIssue("error", f"unknown session field in '{text}'"))
            return "any"
        return SESSION_FIELDS[parts[1]]
    if root == "last":
        if len(parts) != 1:
            issues.append(TemplateIssue("error", f"'last' has no fields, got '{text}'"))
        elif not scope.last:
            issues.append(TemplateIssue(
                "error", "'last' needs an expect that always runs earlier on the same side"))
        return "bytes"
    if scope.modules is not None and root in scope.modules:
        issues.append(TemplateIssue("error", f"'{text}' names a module function; call it with parentheses"))
        return "any"
    issues.append(TemplateIssue("error", f"unknown reference root '{root}'"))
    return "any"


def _resolve_call(call: Call, scope: TemplateScope, issues: list[TemplateIssue]) -> FunctionSig | None:
    if call.module is None:
        sig = CORE_FUNCTIONS.get(call.name)
        if sig is None:
            issues.append(TemplateIssue("error", f"unknown function '{call.name}'"))
        return sig
    if scope.modules is None:
        return None
    if call.module not in scope.modules:
        issues.append(TemplateIssue(
            "error", f"module '{call.module}' is neither '{scope.module_name}' nor a required module"))
        return None
    catalog = scope.modules[call.module]
    if catalog is None:
        issues.append(TemplateIssue(
            "warning", f"cannot check '{call.qualified}', the manifest of module '{call.module}' is not available"))
        return None
    sig = catalog.get(call.name)
    if sig is None:
        issues.append(TemplateIssue(
            "error", f"module '{call.module}' declares no template function '{call.name}' in its manifest"))
    return sig


def _check_call(call: Call, scope: TemplateScope, issues: list[TemplateIssue]) -> str:
    sig = _resolve_call(call, scope, issues)
    arg_types = [(None, _check_node(a, scope, issues), a) for a in call.positional]
    arg_types += [(k, _check_node(a, scope, issues), a) for k, a in call.keywords]
    if sig is None:
        return "bytes" if call.module else "any"
    params = {p.name: p for p in sig.params}
    bound: dict[str, tuple[str, Any]] = {}
    if len(call.positional) > len(sig.params):
        issues.append(TemplateIssue(
            "error", f"function '{call.qualified}' takes at most {len(sig.params)} positional argument(s), "
                     f"got {len(call.positional)}"))
    for param, (_, typ, node) in zip(sig.params, arg_types[:len(call.positional)]):
        bound[param.name] = (typ, node)
    for name, typ, node in arg_types[len(call.positional):]:
        if name not in params:
            issues.append(TemplateIssue("error", f"function '{call.qualified}' has no parameter '{name}'"))
        elif name in bound:
            issues.append(TemplateIssue("error", f"function '{call.qualified}' got parameter '{name}' twice"))
        else:
            bound[name] = (typ, node)
    for param in sig.params:
        if param.name not in bound:
            if param.required:
                issues.append(TemplateIssue(
                    "error", f"function '{call.qualified}' misses required parameter '{param.name}'"))
            continue
        typ, node = bound[param.name]
        if not compatible(param.type, typ):
            issues.append(TemplateIssue(
                "error", f"parameter '{param.name}' of '{call.qualified}' takes {param.type}, got {typ}"))
    if call.qualified == "hex" and "s" in bound:
        node = bound["s"][1]
        if isinstance(node, Lit) and node.type == "text" and not HEX_RE.fullmatch(node.value):
            issues.append(TemplateIssue("error", "hex() needs an even number of hexadecimal digits"))
    return sig.result


def _check_node(node: Any, scope: TemplateScope, issues: list[TemplateIssue]) -> str:
    if isinstance(node, Lit):
        return node.type
    if isinstance(node, Ref):
        return _check_reference(node, scope, issues)
    return _check_call(node, scope, issues)


def check_expression(expr: str, scope: TemplateScope | set[str] | dict[str, str]) -> list[TemplateIssue]:
    """Check one placeholder expression and return its issues."""
    scope = _as_scope(scope)
    if not expr.strip():
        return [TemplateIssue("error", "empty placeholder")]
    try:
        node = parse_expression(expr)
    except TemplateSyntaxError as exc:
        return [TemplateIssue("error", f"cannot parse expression '{expr.strip()}': {exc}")]
    issues: list[TemplateIssue] = []
    _check_node(node, scope, issues)
    return issues


def _as_scope(scope: TemplateScope | set[str] | dict[str, str]) -> TemplateScope:
    if isinstance(scope, TemplateScope):
        return scope
    if isinstance(scope, dict):
        return TemplateScope(vars=dict(scope))
    return TemplateScope(vars={name: "any" for name in scope})


def check_template(text: str, scope: TemplateScope | set[str] | dict[str, str]) -> list[TemplateIssue]:
    """Check every placeholder of a template. A bare set of names gives every
    variable the type any and leaves module calls unchecked."""
    scope = _as_scope(scope)
    issues: list[TemplateIssue] = []
    stripped = PLACEHOLDER.sub("", text)
    if "{{" in stripped or "}}" in stripped:
        issues.append(TemplateIssue("error", "unbalanced '{{' or '}}'"))
    for m in PLACEHOLDER.finditer(text):
        issues.extend(check_expression(m.group(1), scope))
    return issues


def function_catalog(manifest: dict) -> dict[str, FunctionSig]:
    """Build the template function catalog that a module manifest declares."""
    catalog: dict[str, FunctionSig] = {}
    for fn in manifest.get("functions", []) or []:
        params = tuple(Param(p["name"], p["type"], "default" not in p) for p in fn.get("params", []))
        catalog[fn["name"]] = FunctionSig(fn["name"], params)
    return catalog


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


def _check_side_templates(actions: list, path: list, scope: TemplateScope, seen: bool,
                          file: Path, report: Report) -> bool:
    """Check the templates of one action list in order and return whether an
    expect always ran by its end. A loop body may run zero times unless its
    count is a positive integer, and a branch guarantees an expect only when
    every arm does."""
    for i, action in enumerate(actions or []):
        if not isinstance(action, dict) or len(action) != 1:
            continue
        (kind, body), = action.items()
        here = path + [i, kind]
        scope.last = seen
        for key, text in _template_fields(kind, body):
            for issue in check_template(text, scope):
                report.add(issue.severity, file, _pointer(here + ([key] if key else [])), issue.message)
        if kind == "expect":
            seen = True
        elif kind == "loop" and isinstance(body, dict):
            inner = _check_side_templates(body.get("do", []), here + ["do"], scope, seen, file, report)
            count = body.get("count")
            runs = isinstance(count, int) and not isinstance(count, bool) and count >= 1
            seen = seen or (inner and runs)
        elif kind == "branch" and isinstance(body, list):
            arms = [_check_side_templates(arm.get("do", []), here + [j, "do"], scope, seen, file, report)
                    for j, arm in enumerate(body) if isinstance(arm, dict)]
            seen = seen or (bool(arms) and all(arms))
    return seen


def check_scenario(doc: dict, file: Path, report: Report, module: dict | None = None,
                   manifests: dict[str, dict] | None = None) -> None:
    """Check the semantic rules of one scenario.

    module is the manifest of the module that ships the scenario. manifests
    maps module names to manifests and supplies the function catalogs of
    required modules. Without module, module calls stay unchecked.
    """
    flows = doc.get("flows", [])
    var_types = {name: value_type(value) for name, value in (doc.get("vars") or {}).items()}
    # Variables bound by `set` actions are also visible to later templates.
    # Their type depends on the rendered text, so the validator treats it as any.
    for flow in flows:
        for side in ("client", "server"):
            for _, kind, body in _walk_actions(flow.get(side, []), []):
                if kind == "set" and isinstance(body, dict):
                    for name in body:
                        var_types[name] = "any" if name not in var_types else var_types[name]

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

    callable_modules: dict[str, dict[str, FunctionSig] | None] | None = None
    if module is not None:
        known = dict(manifests or {})
        known[module_name] = module
        callable_modules = {name: (function_catalog(known[name]) if name in known else None)
                            for name in allowed_modules}

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
        udp = flow.get("l4", "tcp") == "udp"
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
                if kind == "expect" and isinstance(body, dict):
                    _check_expect(body, udp, path, file, report)
            scope = TemplateScope(vars=var_types, modules=callable_modules, module_name=module_name)
            _check_side_templates(flow.get(side, []), ["flows", i, side], scope, False, file, report)

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


def _check_expect(body: dict, udp: bool, path: list, file: Path, report: Report) -> None:
    """Datagram rules for expect. On UDP one expect consumes one datagram, so
    a fixed size rule must fit within max_datagram."""
    if not udp:
        if "max_datagram" in body:
            report.error(file, _pointer(path + ["max_datagram"]), "max_datagram applies only to UDP flows")
        return
    limit = body.get("max_datagram", MAX_DATAGRAM)
    for key in ("bytes", "min_bytes"):
        size = body.get(key)
        if isinstance(size, int) and not isinstance(size, bool) and size > limit:
            report.error(file, _pointer(path + [key]),
                         f"{key} {size} exceeds the datagram limit of {limit} bytes")
    pattern = body.get("pattern")
    if isinstance(pattern, str) and len(pattern.encode()) > limit:
        report.error(file, _pointer(path + ["pattern"]),
                     f"pattern is longer than the datagram limit of {limit} bytes")


# ---------------------------------------------------------------------------
# Module checks
# ---------------------------------------------------------------------------

@dataclass
class ModuleIndex:
    manifests: dict[str, dict] = field(default_factory=dict)
    scenarios: dict[str, dict[str, dict]] = field(default_factory=dict)
    # Scenarios that passed the schema, as (module name, path, doc). Their
    # semantic checks wait until every manifest is loaded, so calls into a
    # required module find its function catalog.
    pending: list[tuple[str, Path, dict]] = field(default_factory=list)

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

    if name in RESERVED_ROOTS:
        report.error(manifest_path, "/name", f"module name '{name}' is reserved for template references")
    for i, req in enumerate(manifest.get("requires", [])):
        if req["module"] == name:
            report.error(manifest_path, f"/requires/{i}", "module requires itself")
    check_functions(manifest, manifest_path, report)
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
        index.pending.append((name, spath, doc))


def check_index_scenarios(index: ModuleIndex, report: Report) -> None:
    """Run the semantic checks of every scenario that check_module_dir loaded."""
    for name, spath, doc in index.pending:
        check_scenario(doc, spath, report, index.manifests[name], index.manifests)
    index.pending.clear()


def check_functions(manifest: dict, path: Path, report: Report) -> None:
    """Check the template function catalog of a manifest.

    The validator never runs extension code. It checks scenario calls against
    this catalog, and the engine rejects a module whose extension returns a
    different catalog from functions() at load time.
    """
    functions = manifest.get("functions") or []
    hooks = (manifest.get("extension") or {}).get("hooks", [])
    if functions and manifest.get("extension") and "render" not in hooks:
        report.error(path, "/extension/hooks", "a module that declares functions must list the render hook")
    if "render" in hooks and not functions:
        report.warning(path, "/extension/hooks", "the render hook is listed but the manifest declares no functions")
    names: set[str] = set()
    for i, fn in enumerate(functions):
        where = f"/functions/{i}"
        if fn["name"] in names:
            report.error(path, where + "/name", f"duplicate function '{fn['name']}'")
        names.add(fn["name"])
        params: set[str] = set()
        for j, param in enumerate(fn.get("params", [])):
            pwhere = f"{where}/params/{j}"
            if param["name"] in params:
                report.error(path, pwhere + "/name", f"duplicate parameter '{param['name']}' of '{fn['name']}'")
            params.add(param["name"])
            if "default" in param:
                problem = _default_problem(param["type"], param["default"])
                if problem:
                    report.error(path, pwhere + "/default", problem)


def _default_problem(typ: str, default: Any) -> str | None:
    if typ == "any":
        return None
    if typ == "bytes":
        if isinstance(default, str) and HEX_RE.fullmatch(default):
            return None
        return "a bytes default must be a string of hexadecimal digit pairs"
    actual = value_type(default)
    if compatible(typ, actual):
        return None
    return f"default of type {actual} does not match parameter type {typ}"


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
    check_index_scenarios(index, report)
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
