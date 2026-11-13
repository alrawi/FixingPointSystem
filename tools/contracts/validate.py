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
import tempfile
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

# The value of the metric label application that stands for every application
# together (metric catalog 0.3.0), so no module may carry the name.
ALL_APPLICATIONS = "all"

# Template roots that are not module names. A module may not take one of
# these names, because a template could not tell the two apart.
RESERVED_ROOTS = {"vars", "session", "last", "true", "false", "profiles", "dist"}

# The module names that the controller registry refuses at install
# (controller/internal/registry/manifest.go, reservedModuleNames). Since CTL-50 the
# registry refuses every name that the validator reserves: the template roots and
# the application name of the metric catalog.
REGISTRY_RESERVED_NAMES = frozenset(RESERVED_ROOTS | {ALL_APPLICATIONS})

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
    # Scenario contract v0.6 (CON-19). The result type of now() depends on the
    # format literal, see NOW_RESULTS.
    "int_bytes": FunctionSig("int_bytes", (Param("value", "int"), Param("width", "int"),
                                           Param("endianness", "text", False)), "bytes"),
    "now": FunctionSig("now", (Param("format", "text"),), "any"),
}

# Formats of now() and the type each one renders (scenario contract v0.6).
NOW_RESULTS = {"http_date": "text", "epoch_s": "int", "epoch_ms": "int"}

# Widths of int_bytes() in bytes, its byte orders, and the largest value of an
# int, which is a signed 64 bit integer.
INT_BYTES_WIDTHS = range(1, 9)
INT_BYTES_ORDERS = ("big", "little")
INT_MAX = 2**63 - 1

# Distribution functions of scenario contract v0.3 (decision D12 item 18). They
# need no manifest: the engine draws from the seeded generator and rounds to an
# integer, then clamps the result to cap, which defaults to DIST_DEFAULT_CAP.
DIST_DEFAULT_CAP = 2**31 - 1

DIST_FUNCTIONS: dict[str, FunctionSig] = {
    "pareto": FunctionSig("pareto", (Param("alpha", "float"), Param("scale", "float"),
                                     Param("cap", "int", False)), "int"),
    "lognormal": FunctionSig("lognormal", (Param("mu", "float"), Param("sigma", "float"),
                                           Param("cap", "int", False)), "int"),
    "exponential": FunctionSig("exponential", (Param("mean", "float"), Param("cap", "int", False)), "int"),
    "uniform": FunctionSig("uniform", (Param("low", "float"), Param("high", "float")), "int"),
}

# Parameters that a literal argument must keep positive, and the ones that must
# not be negative.
DIST_POSITIVE = {"alpha", "scale", "sigma", "mean", "cap"}
DIST_NON_NEGATIVE = {"low", "high"}

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
    profiles: set[str] = field(default_factory=set)


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
    if root == "profiles":
        if len(parts) != 2:
            issues.append(TemplateIssue("error", f"'{text}' must have the form profiles.<name>"))
        elif parts[1] not in scope.profiles:
            issues.append(TemplateIssue("error", f"undefined profile '{parts[1]}'"))
        return "text"
    if root == "dist":
        issues.append(TemplateIssue("error", f"'{text}' names a distribution function; call it with parentheses"))
        return "any"
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
    if call.module == "dist":
        sig = DIST_FUNCTIONS.get(call.name)
        if sig is None:
            issues.append(TemplateIssue(
                "error", f"unknown distribution '{call.name}', expected one of {', '.join(sorted(DIST_FUNCTIONS))}"))
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
    if call.module == "dist":
        _check_dist_literals(call, bound, issues)
    if call.qualified == "int_bytes":
        _check_int_bytes(bound, issues)
    result = sig.result
    if call.qualified == "now":
        node = bound.get("format", (None, None))[1]
        if isinstance(node, Lit) and node.type == "text":
            if node.value not in NOW_RESULTS:
                issues.append(TemplateIssue(
                    "error", f"unknown format '{node.value}' of now(), expected one of {', '.join(NOW_RESULTS)}"))
            else:
                result = NOW_RESULTS[node.value]
    if call.qualified == "hex" and "s" in bound:
        node = bound["s"][1]
        if isinstance(node, Lit) and node.type == "text" and not HEX_RE.fullmatch(node.value):
            issues.append(TemplateIssue("error", "hex() needs an even number of hexadecimal digits"))
    return result


def _check_int_bytes(bound: dict[str, tuple[str, Any]], issues: list[TemplateIssue]) -> None:
    """Range rules for the literal arguments of int_bytes()."""
    def literal(name: str, kind: str) -> Any:
        node = bound.get(name, (None, None))[1]
        return node.value if isinstance(node, Lit) and node.type == kind else None

    width, value, order = literal("width", "int"), literal("value", "int"), literal("endianness", "text")
    if width is not None and width not in INT_BYTES_WIDTHS:
        issues.append(TemplateIssue("error", "int_bytes() takes a width from 1 to 8 bytes"))
        width = None
    if order is not None and order not in INT_BYTES_ORDERS:
        issues.append(TemplateIssue("error", "int_bytes() takes the endianness 'big' or 'little'"))
    if value is not None:
        limit = min(2 ** (8 * width) - 1, INT_MAX) if width is not None else INT_MAX
        if value < 0 or value > limit:
            issues.append(TemplateIssue(
                "error", f"int_bytes() value {value} does not fit {width or 8} byte(s) as an unsigned integer"))


def _check_dist_literals(call: Call, bound: dict[str, tuple[str, Any]], issues: list[TemplateIssue]) -> None:
    """Range rules for the literal arguments of a distribution function."""
    lits = {name: node.value for name, (_, node) in bound.items()
            if isinstance(node, Lit) and node.type in ("int", "float")}
    for name, value in lits.items():
        if name in DIST_POSITIVE and value <= 0:
            issues.append(TemplateIssue("error", f"parameter '{name}' of '{call.qualified}' must be positive"))
        if name in DIST_NON_NEGATIVE and value < 0:
            issues.append(TemplateIssue("error", f"parameter '{name}' of '{call.qualified}' must not be negative"))
    if "low" in lits and "high" in lits and lits["low"] > lits["high"]:
        issues.append(TemplateIssue("error", "uniform() has low greater than high"))


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

    profile_names = set((doc.get("profiles") or {}).keys())
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
        if udp and "transport" in flow:
            report.error(file, f"/flows/{i}/transport", "transport (sugar for tls_upgrade) applies only to TCP flows")
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
                if kind == "tls_upgrade" and udp:
                    report.error(file, _pointer(path), "tls_upgrade applies only to TCP flows")
            scope = TemplateScope(vars=var_types, modules=callable_modules, module_name=module_name,
                                  profiles=profile_names)
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
# Registry rules
#
# The controller registry refuses a module at install for reasons that the
# schema cannot express. The validator applies the same rules, so that a module
# that validates here installs there (CON-30). Each rule names the registry
# source that it follows, and test_contracts.py compares the lists below with
# those sources, so a change on one side fails the tests until the other follows.
# ---------------------------------------------------------------------------

# controller/internal/registry/license.go: the lower case SPDX identifiers that
# may be combined with GPL-3.0 code, the well known ones that may not, and the
# exceptions that only add permissions.
COMPATIBLE_LICENSES = (
    "GPL-3.0-only", "GPL-3.0-or-later", "GPL-2.0-or-later",
    "LGPL-3.0-only", "LGPL-3.0-or-later", "LGPL-2.1-only", "LGPL-2.1-or-later",
    "AGPL-3.0-only", "AGPL-3.0-or-later",
    "Apache-2.0", "MIT", "MIT-0", "X11", "ISC", "0BSD", "Zlib", "BSL-1.0",
    "BSD-2-Clause", "BSD-3-Clause", "MPL-2.0", "Artistic-2.0",
    "Unlicense", "CC0-1.0", "CC-BY-4.0", "CC-BY-SA-4.0",
    "Unicode-3.0", "Unicode-DFS-2016", "CDLA-Permissive-2.0",
)
INCOMPATIBLE_LICENSES = (
    "GPL-2.0-only", "LGPL-2.0-only", "Apache-1.0", "Apache-1.1", "BSD-4-Clause",
    "CDDL-1.0", "CDDL-1.1", "EPL-1.0", "EPL-2.0", "MPL-1.0", "MPL-1.1",
    "MPL-2.0-no-copyleft-exception", "SSPL-1.0", "BUSL-1.1", "JSON", "OpenSSL",
    "CC-BY-NC-4.0", "CC-BY-ND-4.0", "CC-BY-NC-SA-4.0", "CC-BY-NC-ND-4.0",
    "Commons-Clause", "Elastic-2.0",
)
LICENSE_EXCEPTIONS = ("LLVM-exception", "Classpath-exception-2.0", "GCC-exception-3.1")

_COMPATIBLE = {i.lower(): i for i in COMPATIBLE_LICENSES}
_INCOMPATIBLE = {i.lower(): i for i in INCOMPATIBLE_LICENSES}
_EXCEPTIONS = {i.lower(): i for i in LICENSE_EXCEPTIONS}


class _LicenseUnknown(ValueError):
    """A malformed expression, or a license or exception off the allow list."""


def _license_tokens(expr: str) -> list[str]:
    toks: list[str] = []
    cur: list[str] = []

    def flush() -> None:
        if cur:
            toks.append("".join(cur))
            cur.clear()

    for ch in expr:
        if ch in "()":
            flush()
            toks.append(ch)
        elif ch in " \t\n\r":
            flush()
        else:
            cur.append(ch)
    flush()
    return toks


class _LicenseParser:
    """The recursive descent parser of license.go. Operators are upper case, AND
    binds tighter than OR, and WITH follows a single license."""

    def __init__(self, toks: list[str]) -> None:
        self.toks = toks
        self.pos = 0

    def peek(self) -> str:
        return self.toks[self.pos] if self.pos < len(self.toks) else ""

    def binary(self, op: str, nxt) -> tuple:
        first = nxt()
        args = [first]
        while self.peek() == op:
            self.pos += 1
            args.append(nxt())
        return first if len(args) == 1 else (op, args)

    def expr(self) -> tuple:
        return self.binary("OR", lambda: self.binary("AND", self.term))

    def term(self) -> tuple:
        tok = self.peek()
        if tok == "":
            raise _LicenseUnknown("expression ends early")
        if tok == "(":
            self.pos += 1
            node = self.expr()
            if self.peek() != ")":
                raise _LicenseUnknown("missing closing parenthesis")
            self.pos += 1
            return node
        if tok in (")", "AND", "OR", "WITH"):
            raise _LicenseUnknown(f"unexpected '{tok}'")
        self.pos += 1
        key = tok.lower()
        if key in _COMPATIBLE:
            leaf = ("leaf", _COMPATIBLE[key], True)
        elif key in _INCOMPATIBLE:
            leaf = ("leaf", _INCOMPATIBLE[key], False)
        else:
            raise _LicenseUnknown(f"license '{tok}' is not on the allow list")
        if self.peek() == "WITH":
            self.pos += 1
            exc = self.peek()
            if exc.lower() not in _EXCEPTIONS:
                raise _LicenseUnknown(f"license exception '{exc}' is not on the allow list")
            self.pos += 1
            leaf = ("leaf", leaf[1] + " WITH " + _EXCEPTIONS[exc.lower()], leaf[2])
        return leaf


def _incompatible_license(node: tuple) -> str:
    """The license that makes node incompatible, or an empty string."""
    if node[0] == "leaf":
        return "" if node[2] else node[1]
    op, args = node
    bad = [_incompatible_license(a) for a in args]
    if op == "AND":
        return next((b for b in bad if b), "")
    return "" if "" in bad else bad[0]


def license_problem(expr: str) -> str | None:
    """Return why the registry refuses the SPDX expression expr, or None.

    An expression is compatible with GPL-3.0 code when every AND operand is
    compatible and at least one operand of each OR is. Every license and
    exception must be on the allow list, even in an OR operand that is not
    needed, so a typo never passes (controller/internal/registry/license.go,
    CheckLicense).
    """
    parser = _LicenseParser(_license_tokens(expr))
    try:
        if not parser.toks:
            raise _LicenseUnknown("empty expression")
        node = parser.expr()
        if parser.pos < len(parser.toks):
            raise _LicenseUnknown(f"unexpected '{parser.toks[parser.pos]}'")
    except _LicenseUnknown as exc:
        return f"license '{expr}': {exc}"
    bad = _incompatible_license(node)
    if bad:
        return f"license '{expr}': {bad}: license is not compatible with GPL-3.0"
    return None


# controller/internal/registry/archive.go, entryPath and hasDotSegment: every
# file and directory of a module tree, not only the files that the manifest
# names, uses these characters and has no "." or ".." segment.
TREE_PATH_RE = re.compile(r"[A-Za-z0-9_.\-]+(/[A-Za-z0-9_.\-]+)*")


def tree_problems(root: Path) -> list[tuple[str, str]]:
    """Return (path, problem) for every entry of the module tree under root that
    the registry refuses. The root itself may be a link, as the first party
    targets make it, and a link inside the tree is refused, as the registry, the
    digest, and the agent refuse it (archive.go, digest.go, modules.rs)."""
    found: list[tuple[str, str]] = []

    def walk(directory: Path, prefix: str) -> None:
        for entry in sorted(directory.iterdir(), key=lambda e: e.name):
            rel = prefix + entry.name
            if not TREE_PATH_RE.fullmatch(rel) or entry.name in (".", ".."):
                found.append((rel, "the path has a character or a segment that a manifest cannot name; "
                                   "use letters, digits, '_', '.', and '-' only"))
            if entry.is_symlink():
                found.append((rel, "the module tree holds a symbolic link"))
            elif entry.is_dir():
                walk(entry, rel + "/")
            elif not entry.is_file():
                found.append((rel, "the module tree holds a special file"))

    walk(root, "")
    return found


def names_regular_file(root: Path, rel: str) -> bool:
    """Whether rel is a regular file of the tree and not a link to one, which is
    what the registry asks of every path that the manifest names."""
    path = root / rel
    return path.is_file() and not path.is_symlink()


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
    if name == ALL_APPLICATIONS:
        report.error(manifest_path, "/name",
                     f"module name '{name}' is reserved for the application label value that totals every application")
    problem = license_problem(manifest["license"])
    if problem:
        report.error(manifest_path, "/license", problem)
    for rel, why in tree_problems(mod_dir):
        report.error(mod_dir / rel, "", why)
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
            if item and item.get(attr) and not names_regular_file(mod_dir, item[attr]):
                report.error(manifest_path, f"/{key}" + (f"/{i}" if key != "extension" else "") + f"/{attr}",
                             f"file '{item[attr]}' does not exist")

    scenario_schema = load_schema("scenario.schema.json")
    # Scenarios in scenarios-pending/ use syntax that the engine does not
    # compile yet. They are checked but not indexed, and the manifest does not
    # list them.
    for spath in sorted((mod_dir / "scenarios-pending").glob("*.yaml")):
        doc = load_yaml(spath)
        if schema_errors(scenario_schema, doc, report, spath):
            index.pending.append((name, spath, doc))
    names_known = True
    for i, rel in enumerate(manifest.get("scenarios", [])):
        spath = mod_dir / rel
        if not names_regular_file(mod_dir, rel):
            report.error(manifest_path, f"/scenarios/{i}", f"scenario file '{rel}' does not exist")
            names_known = False
            continue
        doc = load_yaml(spath)
        if not schema_errors(scenario_schema, doc, report, spath):
            names_known = False
            continue
        sname = doc["scenario"]
        if sname in index.scenarios[name]:
            report.error(spath, "/scenario", f"duplicate scenario '{sname}' in module '{name}'")
        index.scenarios[name][sname] = doc
        index.pending.append((name, spath, doc))
    # A key of expected_bytes that names no scenario of the manifest is a typo
    # that the controller would never read. The registry does not refuse it yet,
    # so the validator is the only check. It waits when a scenario file failed
    # above, because that scenario's name is then unknown.
    if names_known:
        for key in manifest.get("expected_bytes", {}):
            if key not in index.scenarios[name]:
                report.error(manifest_path, f"/expected_bytes/{key}",
                             f"expected_bytes names '{key}', which is not the name of a scenario in 'scenarios'")


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

    _check_distributions(doc, file, report)


def _check_distributions(doc: dict, file: Path, report: Report) -> None:
    """Semantic rules of the arrival and size distributions (test configuration
    v0.3). The schema fixes the shapes, and these rules relate two parameters."""
    profile = doc["profile"]
    sites: list[tuple[str, dict]] = []
    scopes = [("/profile", profile)] + [(f"/profile/mix/{i}", e) for i, e in enumerate(profile["mix"])]
    for where, scope in scopes:
        if "arrival" in scope:
            sites.append((where + "/arrival", scope["arrival"]))
        for direction, dist in (scope.get("size") or {}).items():
            sites.append((f"{where}/size/{direction}", dist))
    for where, dist in sites:
        for low, high in (("min_ms", "max_ms"), ("min", "max")):
            if low in dist and dist[low] > dist[high]:
                report.error(file, where, f"uniform distribution has {low} greater than {high}")


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


# Example tests that name a scenario of a first party module under modules/. The
# example modules of contracts/examples/modules/ have their own scenarios, so
# these tests are validated against the first party module they name (CON-15).
FIRST_PARTY_TESTS = {"dns_zone.yaml": "dns"}


def run_first_party(tests: list[Path], catalog_path: Path) -> Report:
    """Validate example tests against the first party modules that they name."""
    report = Report()
    with tempfile.TemporaryDirectory() as tmp:
        for name in sorted({FIRST_PARTY_TESTS[t.name] for t in tests}):
            (Path(tmp) / name).symlink_to(REPO / "modules" / name)
        sub = run([Path(tmp)], tests, None, catalog_path)
        report.findings.extend(sub.findings)
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
    default_tests = args.tests is None and args.modules is None
    tests = args.tests if args.tests is not None else sorted((EXAMPLES / "tests").glob("*.yaml"))
    first_party = [t for t in tests if default_tests and t.name in FIRST_PARTY_TESTS]
    tests = [t for t in tests if t not in first_party]
    report = run(modules, tests, None if args.no_openapi else OPENAPI, args.catalog)
    if first_party:
        report.findings.extend(run_first_party(first_party, args.catalog).findings)
    for finding in report.findings:
        if not args.quiet or finding.severity == "error":
            print(finding)
    n_err, n_warn = len(report.errors), len(report.findings) - len(report.errors)
    print(f"contracts: {n_err} error(s), {n_warn} warning(s)")
    return 1 if n_err else 0


if __name__ == "__main__":
    sys.exit(main())
