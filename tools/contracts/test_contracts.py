"""Tests for the contract validator. Run with: python3 -m pytest tools/contracts"""
from __future__ import annotations

import copy
import shutil
import sys
from pathlib import Path

import pytest
import yaml

sys.path.insert(0, str(Path(__file__).parent))
import validate as v  # noqa: E402

EXAMPLES = v.EXAMPLES


def messages(report: v.Report, severity: str = "error") -> list[str]:
    return [f.message for f in report.findings if f.severity == severity]


@pytest.fixture()
def http_manifest() -> dict:
    return v.load_yaml(EXAMPLES / "modules/http1/manifest.yaml")


@pytest.fixture()
def web_get() -> dict:
    return v.load_yaml(EXAMPLES / "modules/http1/scenarios/web_get.yaml")


@pytest.fixture()
def smoke() -> dict:
    return v.load_yaml(EXAMPLES / "tests/http_smoke.yaml")


@pytest.fixture()
def index() -> v.ModuleIndex:
    idx = v.ModuleIndex()
    report = v.Report()
    for mod_dir in sorted((EXAMPLES / "modules").iterdir()):
        v.check_module_dir(mod_dir, report, idx)
    v.check_index_scenarios(idx, report)
    assert not report.errors
    return idx


@pytest.fixture()
def dns_manifest() -> dict:
    return v.load_yaml(EXAMPLES / "modules/dns/manifest.yaml")


@pytest.fixture()
def resolve() -> dict:
    return v.load_yaml(EXAMPLES / "modules/dns/scenarios/resolve.yaml")


@pytest.fixture()
def web_page() -> dict:
    return v.load_yaml(EXAMPLES / "modules/http1/scenarios/web_page.yaml")


def check_scenario(doc: dict, manifest: dict) -> v.Report:
    report = v.Report()
    assert v.schema_errors(v.load_schema("scenario.schema.json"), doc, report, Path("s.yaml")), messages(report)
    v.check_scenario(doc, Path("s.yaml"), report, manifest)
    return report


def test_examples_are_valid():
    assert v.main(["--quiet"]) == 0


def test_schemas_are_valid_json_schema():
    for name in ("module-manifest.schema.json", "scenario.schema.json", "test-config.schema.json"):
        v.Draft202012Validator.check_schema(v.load_schema(name))


def test_openapi_covers_required_operations():
    report = v.Report()
    v.check_openapi(v.OPENAPI, report)
    assert not report.errors, messages(report)


# --- template grammar -------------------------------------------------------

@pytest.mark.parametrize("text", [
    "plain", "{{ vars.host }}", "{{ fill(vars.size) }}", "{{ fill(10) }}",
    "{{ hex('0a0B') }}", "{{ rand_int(1, 6) }}", "{{ session.id }} and {{ session.flow }}",
])
def test_template_accepts(text):
    assert [i for i in v.check_template(text, {"host", "size"}) if i.severity == "error"] == []


@pytest.mark.parametrize("text,needle", [
    ("{{ vars.missing }}", "undefined variable"),
    ("{{ nope(1) }}", "unknown function"),
    ("{{ fill(1, 2) }}", "at most 1 positional argument"),
    ("{{ hex('abc') }}", "even number"),
    ("{{ session.secret }}", "unknown session field"),
    ("{{ other.x }}", "unknown reference root"),
    ("{{ }}", "empty placeholder"),
    ("{{ vars.host ", "unbalanced"),
    ("{{ 1 + 2 }}", "cannot parse"),
    ("{{ fill('x') }}", "takes int, got text"),
    ("{{ rand_int(1) }}", "misses required parameter 'b'"),
    ("{{ fill(m=3) }}", "has no parameter 'm'"),
    ("{{ hex(s='zz') }}", "even number"),
    ("{{ a.b.c(1) }}", "<module>.<function>"),
    ("{{ fill(1,) }}", "cannot parse"),
    ("{{ fill(n=1, 2) }}", "positional argument follows a keyword"),
    ("{{ hex('ab) }}", "unterminated string"),
    ("{{ last.size }}", "'last' has no fields"),
])
def test_template_rejects(text, needle):
    issues = v.check_template(text, {"host"})
    assert any(needle in i.message for i in issues if i.severity == "error"), issues


def test_module_call_without_manifest_is_unchecked():
    # The plan checker validates scenario sources without their manifest.
    assert v.check_template("{{ http1.request('GET', '/') }}", set()) == []


@pytest.mark.parametrize("text,node", [
    ("dns.query(name=vars.host, qtype='A')",
     v.Call("dns", "query", [], [("name", v.Ref(["vars", "host"])), ("qtype", v.Lit("text", "A"))])),
    ("http1.response(200, body=fill(vars.n))",
     v.Call("http1", "response", [v.Lit("int", 200)],
            [("body", v.Call(None, "fill", [v.Ref(["vars", "n"])], []))])),
    ("last", v.Ref(["last"])),
    ("-7", v.Lit("int", -7)),
    ("2.5", v.Lit("float", 2.5)),
    ("false", v.Lit("boolean", False)),
    ("f()", v.Call(None, "f", [], [])),
])
def test_parse_expression(text, node):
    assert v.parse_expression(text) == node


def test_call_depth_limit():
    inner = "1"
    for _ in range(v.MAX_CALL_DEPTH):
        inner = f"fill({inner})"
    assert v.parse_expression(inner) is not None
    with pytest.raises(v.TemplateSyntaxError, match="nest deeper"):
        v.parse_expression(f"fill({inner})")


@pytest.mark.parametrize("param,arg,ok", [
    ("int", "int", True), ("float", "int", True), ("int", "float", False),
    ("bytes", "text", True), ("text", "bytes", False), ("any", "bytes", True),
    ("boolean", "any", True), ("boolean", "int", False),
])
def test_type_compatibility(param, arg, ok):
    assert v.compatible(param, arg) is ok


def test_variable_types_come_from_vars():
    scope = v.TemplateScope(vars={"port": "int", "host": "text"})
    assert not v.check_template("{{ fill(vars.port) }}", scope)
    issues = v.check_template("{{ fill(vars.host) }}", scope)
    assert any("takes int, got text" in i.message for i in issues)


# --- module calls -------------------------------------------------------------

def dns_scope(manifest: dict, last: bool = True) -> v.TemplateScope:
    return v.TemplateScope(vars={"host": "text", "port": "int"}, module_name="dns", last=last,
                           modules={"dns": v.function_catalog(manifest)})


@pytest.mark.parametrize("text", [
    "{{ dns.query(name=vars.host, qtype='A') }}",
    "{{ dns.query(vars.host) }}",
    "{{ dns.query(vars.host, 'AAAA') }}",
    "{{ dns.answer_ip(last) }}",
    "{{ dns.response(last, '192.0.2.7', ttl=vars.port) }}",
    "{{ dns.response(query=hex('00'), address='192.0.2.7') }}",
])
def test_module_call_accepts(text, dns_manifest):
    assert v.check_template(text, dns_scope(dns_manifest)) == []


@pytest.mark.parametrize("text,needle", [
    ("{{ dns.lookup(vars.host) }}", "declares no template function 'lookup'"),
    ("{{ dns.query() }}", "misses required parameter 'name'"),
    ("{{ dns.query(vars.host, name=vars.host) }}", "got parameter 'name' twice"),
    ("{{ dns.query(vars.host, 'A', 3) }}", "at most 2 positional"),
    ("{{ dns.query(name=vars.host, class='IN') }}", "has no parameter 'class'"),
    ("{{ dns.query(name=vars.port) }}", "takes text, got int"),
    ("{{ dns.response(last, '192.0.2.7', ttl=dns.query(vars.host)) }}", "takes int, got bytes"),
    ("{{ dns.response(last, address=dns.answer_ip(last)) }}", "takes text, got bytes"),
    ("{{ smtp.helo() }}", "neither 'dns' nor a required module"),
    ("{{ dns.query }}", "call it with parentheses"),
])
def test_module_call_rejects(text, needle, dns_manifest):
    issues = v.check_template(text, dns_scope(dns_manifest))
    assert any(needle in i.message for i in issues if i.severity == "error"), issues


def test_required_module_without_manifest_is_a_warning(web_get, http_manifest):
    http_manifest["requires"] = [{"module": "dns", "version": ">=0.1"}]
    web_get["flows"][0]["client"].insert(1, {"send": "{{ dns.query(name=vars.host) }}"})
    report = check_scenario(web_get, http_manifest)
    assert not report.errors
    assert any("manifest of module 'dns' is not available" in m for m in messages(report, "warning"))


def test_required_module_calls_are_checked(web_get, http_manifest, dns_manifest):
    http_manifest["requires"] = [{"module": "dns", "version": ">=0.1"}]
    web_get["flows"][0]["client"].insert(1, {"send": "{{ dns.query(name=vars.host) }}"})
    web_get["flows"][0]["client"].insert(2, {"send": "{{ dns.query(host=vars.host) }}"})
    report = v.Report()
    v.check_scenario(web_get, Path("s.yaml"), report, http_manifest, {"dns": dns_manifest})
    assert [f.path for f in report.errors] == ["/flows/0/client/2/send", "/flows/0/client/2/send"]


def test_examples_use_module_calls(web_page, resolve):
    for doc in (web_page, resolve):
        sends = [b for _, k, b in v._walk_actions(doc["flows"][0]["client"], []) if k == "send"]
        assert any("." in v.parse_expression(m.group(1)).qualified for s in sends
                   for m in v.PLACEHOLDER.finditer(s))


def test_example_module_call_errors_fail_the_validator(web_page, http_manifest):
    web_page["flows"][0]["server"][2]["send"] = "{{ http1.response('200') }}"
    assert any("takes int, got text" in m for m in messages(check_scenario(web_page, http_manifest)))


# --- manifest function catalog ----------------------------------------------

def manifest_report(manifest: dict) -> v.Report:
    report = v.Report()
    if v.schema_errors(v.load_schema("module-manifest.schema.json"), manifest, report, Path("m")):
        v.check_functions(manifest, Path("m"), report)
    return report


def test_example_function_catalogs_are_valid(dns_manifest, http_manifest):
    assert not manifest_report(dns_manifest).findings
    assert not manifest_report(http_manifest).findings
    sig = v.function_catalog(dns_manifest)["query"]
    assert [(p.name, p.type, p.required) for p in sig.params] == [("name", "text", True), ("qtype", "text", False)]


@pytest.mark.parametrize("change,needle", [
    (lambda m: m["functions"].append(dict(m["functions"][0])), "duplicate function 'query'"),
    (lambda m: m["functions"][0]["params"].append({"name": "name", "type": "text"}), "duplicate parameter"),
    (lambda m: m["functions"][2]["params"][2].update(default="long"), "does not match parameter type int"),
    (lambda m: m["functions"][0]["params"].append({"name": "raw", "type": "bytes", "default": "xyz"}),
     "hexadecimal digit pairs"),
    (lambda m: m.update(extension={"wasm": "manifest.yaml", "hooks": ["on_data"]}), "must list the render hook"),
])
def test_function_catalog_rejects(change, needle, dns_manifest):
    change(dns_manifest)
    assert any(needle in m for m in messages(manifest_report(dns_manifest))), messages(manifest_report(dns_manifest))


def test_function_catalog_schema_rejects_unknown_type(dns_manifest):
    dns_manifest["functions"][0]["params"][0]["type"] = "string"
    assert manifest_report(dns_manifest).errors


def test_render_hook_without_functions_is_a_warning(dns_manifest):
    del dns_manifest["functions"]
    dns_manifest["extension"] = {"wasm": "manifest.yaml", "hooks": ["render"]}
    report = manifest_report(dns_manifest)
    assert not report.errors
    assert any("declares no functions" in m for m in messages(report, "warning"))


def test_reserved_module_name(tmp_path):
    mod = tmp_path / "last"
    shutil.copytree(EXAMPLES / "modules/dns", mod)
    manifest = v.load_yaml(mod / "manifest.yaml")
    manifest["name"] = "last"
    (mod / "manifest.yaml").write_text(yaml.safe_dump(manifest))
    report = v.Report()
    v.check_module_dir(mod, report, v.ModuleIndex())
    assert any("reserved for template references" in m for m in messages(report))


# --- the reference last -------------------------------------------------------

def test_last_after_expect_is_valid(resolve, dns_manifest):
    assert not check_scenario(resolve, dns_manifest).findings


def test_last_before_any_expect_fails(resolve, dns_manifest):
    client = resolve["flows"][0]["client"]
    client.insert(1, {"set": {"early": "{{ dns.answer_ip(last) }}"}})
    report = check_scenario(resolve, dns_manifest)
    assert [f.path for f in report.errors] == ["/flows/0/client/1/set/early"]


def test_last_does_not_cross_sides(resolve, dns_manifest):
    # The client expect does not feed the server side.
    server = resolve["flows"][0]["server"]
    del server[1]
    assert any("'last' needs an expect" in m for m in messages(check_scenario(resolve, dns_manifest)))


@pytest.mark.parametrize("block,ok", [
    ({"loop": {"count": 2, "do": [{"expect": {"min_bytes": 1}}]}}, True),
    ({"loop": {"count": 0, "do": [{"expect": {"min_bytes": 1}}]}}, False),
    ({"loop": {"count": "{{ vars.server_port }}", "do": [{"expect": {"min_bytes": 1}}]}}, False),
    ({"branch": [{"p": 0.5, "do": [{"expect": {"min_bytes": 1}}]},
                 {"p": 0.5, "do": [{"expect": {"bytes": 4}}]}]}, True),
    ({"branch": [{"p": 0.5, "do": [{"expect": {"min_bytes": 1}}]},
                 {"p": 0.5, "do": [{"delay": {"ms": 1}}]}]}, False),
])
def test_last_after_loop_and_branch(block, ok, resolve, dns_manifest):
    client = resolve["flows"][0]["client"]
    client[2] = block
    report = check_scenario(resolve, dns_manifest)
    assert (not report.errors) is ok, messages(report)


def test_last_inside_loop_needs_expect_before_it(resolve, dns_manifest):
    client = resolve["flows"][0]["client"]
    client[2:4] = [{"loop": {"count": 3, "do": [
        {"set": {"server_ip": "{{ dns.answer_ip(last) }}"}},
        {"expect": {"min_bytes": 12}},
    ]}}]
    assert any("'last' needs an expect" in m for m in messages(check_scenario(resolve, dns_manifest)))


# --- UDP datagram rules -------------------------------------------------------

def test_max_datagram_only_on_udp(web_get, http_manifest):
    web_get["flows"][0]["client"][1]["loop"]["do"][1]["expect"]["max_datagram"] = 512
    assert any("only to UDP" in m for m in messages(check_scenario(web_get, http_manifest)))


@pytest.mark.parametrize("expect,needle", [
    ({"bytes": 600, "max_datagram": 512}, "bytes 600 exceeds the datagram limit of 512"),
    ({"min_bytes": 70000}, "min_bytes 70000 exceeds the datagram limit of 65507"),
    ({"pattern": "x" * 20, "max_datagram": 16}, "pattern is longer"),
])
def test_udp_expect_must_fit_one_datagram(expect, needle, resolve, dns_manifest):
    resolve["flows"][0]["client"][2] = {"expect": expect}
    assert any(needle in m for m in messages(check_scenario(resolve, dns_manifest)))


def test_schema_bounds_max_datagram(resolve):
    resolve["flows"][0]["client"][2]["expect"]["max_datagram"] = 65508
    report = v.Report()
    assert not v.schema_errors(v.load_schema("scenario.schema.json"), resolve, report, Path("s.yaml"))


# --- scenario semantics -----------------------------------------------------

def test_duplicate_flow_ids(web_get, http_manifest):
    web_get["flows"].append(copy.deepcopy(web_get["flows"][0]))
    assert any("duplicate flow id" in m for m in messages(check_scenario(web_get, http_manifest)))


def test_after_unknown_and_cycle(web_get, http_manifest):
    a = web_get["flows"][0]
    b = copy.deepcopy(a)
    b["id"] = "second"
    a["after"] = "second"
    b["after"] = ["page", "ghost"]
    web_get["flows"].append(b)
    msgs = messages(check_scenario(web_get, http_manifest))
    assert any("unknown flow 'ghost'" in m for m in msgs)
    assert any("dependency cycle" in m for m in msgs)


def test_spawn_requires_spawned_flag(web_get, http_manifest):
    data = copy.deepcopy(web_get["flows"][0])
    data["id"] = "data"
    web_get["flows"].append(data)
    web_get["flows"][0]["client"].insert(1, {"spawn": {"flow": "data"}})
    assert any("spawned: true" in m for m in messages(check_scenario(web_get, http_manifest)))
    data["spawned"] = True
    assert not check_scenario(web_get, http_manifest).errors


def test_branch_probabilities_must_sum_to_one(web_get, http_manifest):
    web_get["flows"][0]["client"].insert(1, {"branch": [
        {"p": 0.5, "do": [{"delay": {"ms": 1}}]},
        {"p": 0.4, "do": [{"delay": {"ms": 2}}]},
    ]})
    assert any("sum to 0.9" in m for m in messages(check_scenario(web_get, http_manifest)))


def test_raw_reserved_for_strike_modules(web_get, http_manifest):
    web_get["flows"][0]["client"].insert(1, {"raw": {"hex": "deadbeef"}})
    assert any("reserved" in m for m in messages(check_scenario(web_get, http_manifest)))
    strike = dict(http_manifest, type="strike")
    assert not check_scenario(web_get, strike).errors


def test_client_must_start_with_connect(web_get, http_manifest):
    web_get["flows"][0]["client"].insert(0, {"delay": {"ms": 5}})
    assert any("must start with 'connect'" in m for m in messages(check_scenario(web_get, http_manifest)))


def test_flow_module_must_be_owner_or_required(web_get, http_manifest):
    web_get["flows"][0]["module"] = "smtp"
    assert any("neither" in m for m in messages(check_scenario(web_get, http_manifest)))


def test_uniform_delay_bounds(web_get, http_manifest):
    web_get["flows"][0]["client"].insert(1, {"delay": {"dist": "uniform", "min_ms": 9, "max_ms": 3}})
    assert any("min_ms greater" in m for m in messages(check_scenario(web_get, http_manifest)))


def test_schema_rejects_two_keys_in_one_action(web_get):
    web_get["flows"][0]["client"][0] = {"connect": {"port": 80}, "send": "x"}
    report = v.Report()
    assert not v.schema_errors(v.load_schema("scenario.schema.json"), web_get, report, Path("s.yaml"))


def test_schema_rejects_expect_without_stop_rule(web_get):
    web_get["flows"][0]["client"][1]["loop"]["do"][1] = {"expect": {"timeout_ms": 5}}
    report = v.Report()
    assert not v.schema_errors(v.load_schema("scenario.schema.json"), web_get, report, Path("s.yaml"))


def test_set_binds_variables_for_later_templates(web_get, http_manifest):
    web_get["flows"][0]["client"].insert(1, {"set": {"token": "{{ rand_int(1, 9) }}"}})
    web_get["flows"][0]["client"].insert(2, {"send": "T={{ vars.token }}"})
    assert not check_scenario(web_get, http_manifest).errors


# --- test configuration semantics -------------------------------------------

def check_test(doc: dict, index: v.ModuleIndex) -> v.Report:
    report = v.Report()
    v.check_test_config(doc, Path("t.yaml"), report, index)
    return report


def test_pool_count_exceeds_cidr(smoke, index):
    smoke["network"]["host_pools"][0]["count"] = 300
    assert any("exceeds" in m for m in messages(check_test(smoke, index)))


def test_gateway_outside_cidr(smoke, index):
    smoke["network"]["host_pools"][0]["gateway"] = "10.9.9.9"
    assert any("outside" in m for m in messages(check_test(smoke, index)))


def test_unknown_port_and_scenario(smoke, index):
    smoke["network"]["host_pools"][0]["port"] = "nope"
    smoke["profile"]["mix"][0]["scenario"] = "http1/missing"
    msgs = messages(check_test(smoke, index))
    assert any("unknown port" in m for m in msgs)
    assert any("unknown scenario" in m for m in msgs)


def test_bare_scenario_name_resolves(smoke, index):
    smoke["profile"]["mix"][0]["scenario"] = "web_get"
    assert not check_test(smoke, index).errors


def test_params_checked_against_module_schema(smoke, index):
    smoke["profile"]["mix"][0]["params"] = {"requests": 0, "bogus": 1}
    msgs = messages(check_test(smoke, index))
    assert any("less than the minimum" in m for m in msgs)
    assert any("bogus" in m for m in msgs)


def test_app_sim_needs_both_roles(smoke, index):
    smoke["network"]["host_pools"] = [smoke["network"]["host_pools"][0]]
    assert any("app_sim needs" in m for m in messages(check_test(smoke, index)))


def test_pool_role_mismatch(smoke, index):
    smoke["profile"]["mix"][0]["client_pool"] = "servers"
    assert any("has role 'server'" in m for m in messages(check_test(smoke, index)))


def test_yaml_exponent_without_sign_is_rejected(smoke, index):
    # YAML 1.1 reads 1.0e9 as a string; the schema must reject it.
    smoke["load"]["objective"]["value"] = yaml.safe_load("v: 1.0e9")["v"]
    assert any("not of type 'number'" in m for m in messages(check_test(smoke, index)))


# --- module directory checks ------------------------------------------------

def test_module_dir_name_and_missing_files(tmp_path):
    mod = tmp_path / "wrongname"
    shutil.copytree(EXAMPLES / "modules/http1", mod)
    manifest = v.load_yaml(mod / "manifest.yaml")
    manifest["scenarios"].append("scenarios/absent.yaml")
    manifest["fixtures"] = [{"capture": "fixtures/none.pcapng", "expect": {"ndpi": ["HTTP"]}}]
    (mod / "manifest.yaml").write_text(yaml.safe_dump(manifest))
    report = v.Report()
    v.check_module_dir(mod, report, v.ModuleIndex())
    msgs = messages(report)
    assert any("differs from directory" in m for m in msgs)
    assert any("absent.yaml" in m for m in msgs)
    assert any("none.pcapng" in m for m in msgs)


def test_capture_provenance_requires_scrub(http_manifest):
    http_manifest["provenance"] = {"source": "capture", "consent": "lab capture", "scrubbed": False}
    report = v.Report()
    assert not v.schema_errors(v.load_schema("module-manifest.schema.json"), http_manifest, report, Path("m"))


def test_openapi_null_value_is_reported(tmp_path):
    doc = v.load_yaml(v.OPENAPI)
    doc["components"]["schemas"]["Agent"]["properties"]["state"] = None
    broken = v.OPENAPI.parent / "_test_null.yaml"
    try:
        broken.write_text(yaml.safe_dump(doc))
        report = v.Report()
        v.check_openapi(broken, report)
    finally:
        broken.unlink(missing_ok=True)
    assert any("null value" in m for m in messages(report))


# --- metric catalog ---------------------------------------------------------

@pytest.fixture()
def catalog_doc() -> dict:
    return v.load_yaml(v.CATALOG)


def load_catalog_doc(doc: dict, tmp_path: Path) -> tuple[v.MetricCatalog, v.Report]:
    path = tmp_path / "catalog.yaml"
    path.write_text(yaml.safe_dump(doc))
    report = v.Report()
    return v.load_catalog(path, report), report


def test_catalog_is_valid_and_covers_four_levels():
    report = v.Report()
    catalog = v.load_catalog(v.CATALOG, report)
    assert not report.findings, messages(report)
    levels = {m["level"] for m in catalog.metrics.values()}
    assert levels == {"application", "scenario", "flow_action", "transport", "port_host"}
    assert {"counter", "gauge", "histogram"} <= {m["type"] for m in catalog.metrics.values()}


def test_catalog_schema_is_valid_json_schema():
    import json
    v.Draft202012Validator.check_schema(json.loads(v.CATALOG_SCHEMA.read_text()))


def test_example_criteria_metrics_exist():
    catalog = v.default_catalog()
    assert catalog.check_reference("sessions.failed_ratio") is None
    assert catalog.check_reference("mix.max_abs_error_pp") is None


def test_unknown_criteria_metric_fails(smoke, index):
    smoke["criteria"].append({"metric": "sessions.vanished_ratio", "op": "<", "value": 1})
    msgs = messages(check_test(smoke, index))
    assert any("unknown metric 'sessions.vanished_ratio'" in m for m in msgs)


def test_unknown_criteria_metric_fails_the_cli(tmp_path):
    doc = v.load_yaml(EXAMPLES / "tests/http_smoke.yaml")
    doc["criteria"] = [{"metric": "made.up", "op": "==", "value": 0}]
    bad = tmp_path / "bad.yaml"
    bad.write_text(yaml.safe_dump(doc))
    assert v.main(["--quiet", "--no-openapi", "--tests", str(bad)]) == 1


@pytest.mark.parametrize("ref,needle", [
    ("sessions.response_time.p99", None),
    ("tcp.rtt.mean", None),
    ("sessions.response_time", "needs a statistic suffix"),
    ("sessions.failed.p99", "applies only to histograms"),
    ("sessions.response_time.p42", "unknown metric"),
])
def test_histogram_statistics(ref, needle):
    problem = v.default_catalog().check_reference(ref)
    assert (problem is None) if needle is None else (needle in problem)


def test_catalog_duplicate_and_undeclared_label(catalog_doc, tmp_path):
    catalog_doc["metrics"].append(copy.deepcopy(catalog_doc["metrics"][0]))
    catalog_doc["metrics"][1]["labels"] = ["application", "colour"]
    _, report = load_catalog_doc(catalog_doc, tmp_path)
    msgs = messages(report)
    assert any("duplicate metric" in m for m in msgs)
    assert any("label 'colour' is not declared" in m for m in msgs)


def test_catalog_formula_references(catalog_doc, tmp_path):
    ratio = next(m for m in catalog_doc["metrics"] if m["name"] == "sessions.failed_ratio")
    ratio["formula"] = "sessions.failed / sessions.missing + sessions.response_time"
    _, report = load_catalog_doc(catalog_doc, tmp_path)
    msgs = messages(report)
    assert any("unknown metric 'sessions.missing'" in m for m in msgs)
    assert any("histogram 'sessions.response_time'" in m for m in msgs)


def test_catalog_formula_requires_gauge(catalog_doc, tmp_path):
    ratio = next(m for m in catalog_doc["metrics"] if m["name"] == "sessions.failed_ratio")
    ratio["type"] = "counter"
    _, report = load_catalog_doc(catalog_doc, tmp_path)
    assert report.errors


def test_catalog_name_collides_with_histogram_statistic(catalog_doc, tmp_path):
    catalog_doc["metrics"].append({"name": "tcp.rtt.p99", "type": "gauge", "unit": "us",
                                   "level": "transport", "labels": [], "description": "x"})
    _, report = load_catalog_doc(catalog_doc, tmp_path)
    assert any("collides with a statistic" in m for m in messages(report))


def test_openapi_unknown_metric_fails():
    doc = v.load_yaml(v.OPENAPI)
    doc["components"]["schemas"]["RunSummary"]["properties"]["bytes_tx"]["x-fps-metric"] = "bytes.sent"
    report = v.Report()
    v.check_openapi_metrics(doc, v.OPENAPI, report, v.default_catalog())
    assert any("unknown metric 'bytes.sent'" in m for m in messages(report))


def test_openapi_summary_property_needs_metric():
    doc = v.load_yaml(v.OPENAPI)
    doc["components"]["schemas"]["RunSummary"]["properties"]["goodput_bps"] = {"type": "number"}
    report = v.Report()
    v.check_openapi_metrics(doc, v.OPENAPI, report, v.default_catalog())
    assert any("goodput_bps" in f.path for f in report.errors)
