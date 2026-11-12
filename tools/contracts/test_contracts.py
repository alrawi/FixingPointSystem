"""Tests for the contract validator. Run with: python3 -m pytest tools/contracts"""
from __future__ import annotations

import copy
import os
import re
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
    (lambda m: m.update(extension={"wasm": "extension.wasm", "hooks": ["on_data"]}), "must list the render hook"),
])
def test_function_catalog_rejects(change, needle, dns_manifest):
    change(dns_manifest)
    assert any(needle in m for m in messages(manifest_report(dns_manifest))), messages(manifest_report(dns_manifest))


def test_function_catalog_schema_rejects_unknown_type(dns_manifest):
    dns_manifest["functions"][0]["params"][0]["type"] = "string"
    assert manifest_report(dns_manifest).errors


def test_render_hook_without_functions_is_a_warning(dns_manifest):
    del dns_manifest["functions"]
    dns_manifest["extension"] = {"wasm": "extension.wasm", "hooks": ["render"]}
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


# --- tls_upgrade -------------------------------------------------------------

STARTTLS = {"tls_upgrade": {"profile": "strict", "server_name": "mail.example.test"}}


def test_tls_upgrade_validates_in_a_tcp_flow(web_get, http_manifest):
    flow = web_get["flows"][0]
    flow["client"].insert(1, copy.deepcopy(STARTTLS))
    flow["server"].insert(1, {"tls_upgrade": {"profile": "strict", "role": "server"}})
    assert not check_scenario(web_get, http_manifest).errors


def test_tls_upgrade_is_checked_inside_a_loop(web_get, http_manifest):
    web_get["flows"][0]["client"][1]["loop"]["do"].insert(0, copy.deepcopy(STARTTLS))
    assert not check_scenario(web_get, http_manifest).errors


def test_tls_upgrade_in_a_udp_flow_is_an_error(resolve, dns_manifest):
    assert resolve["flows"][0]["l4"] == "udp"
    resolve["flows"][0]["client"].insert(1, copy.deepcopy(STARTTLS))
    assert any("tls_upgrade applies only to TCP" in m for m in messages(check_scenario(resolve, dns_manifest)))


def test_tls_upgrade_nested_in_a_udp_flow_is_an_error(resolve, dns_manifest):
    resolve["flows"][0]["client"].insert(1, {"loop": {"count": 2, "do": [copy.deepcopy(STARTTLS)]}})
    assert any("tls_upgrade applies only to TCP" in m for m in messages(check_scenario(resolve, dns_manifest)))


@pytest.mark.parametrize("body", [
    {},
    {"profile": ""},
    {"profile": "p", "role": "peer"},
    {"profile": "p", "server_name": ""},
    {"profile": "p", "extra": 1},
])
def test_schema_rejects_a_malformed_tls_upgrade(body, web_get):
    web_get["flows"][0]["client"].insert(1, {"tls_upgrade": body})
    report = v.Report()
    assert not v.schema_errors(v.load_schema("scenario.schema.json"), web_get, report, Path("s.yaml"))


# --- transport, the sugar for a leading tls_upgrade (scenario contract v0.5) --

TRANSPORT = {"module": "tls", "profile": "strict", "server_name": "www.example.test"}


def test_transport_validates_in_a_tcp_flow(web_get, http_manifest):
    web_get["flows"][0]["transport"] = copy.deepcopy(TRANSPORT)
    report = v.Report()
    assert v.schema_errors(v.load_schema("scenario.schema.json"), web_get, report, Path("s.yaml"))
    assert not check_scenario(web_get, http_manifest).errors


def test_transport_without_server_name_validates(web_get, http_manifest):
    web_get["flows"][0]["transport"] = {"module": "tls", "profile": "strict"}
    assert not check_scenario(web_get, http_manifest).errors


def test_transport_composes_with_tls_upgrade(web_get, http_manifest):
    flow = web_get["flows"][0]
    flow["transport"] = copy.deepcopy(TRANSPORT)
    flow["client"].insert(1, copy.deepcopy(STARTTLS))
    assert not check_scenario(web_get, http_manifest).errors


def test_transport_in_a_udp_flow_is_an_error(resolve, dns_manifest):
    assert resolve["flows"][0]["l4"] == "udp"
    resolve["flows"][0]["transport"] = copy.deepcopy(TRANSPORT)
    report = check_scenario(resolve, dns_manifest)
    assert any(f.path == "/flows/0/transport" and "applies only to TCP" in f.message for f in report.errors)


@pytest.mark.parametrize("body", [
    {},
    {"module": "tls"},
    {"profile": "p"},
    {"module": "quic", "profile": "p"},
    {"module": "tls", "profile": ""},
    {"module": "tls", "profile": "p", "server_name": ""},
    {"module": "tls", "profile": "p", "role": "client"},
    "tls",
])
def test_schema_rejects_a_malformed_transport(body, web_get):
    web_get["flows"][0]["transport"] = body
    report = v.Report()
    assert not v.schema_errors(v.load_schema("scenario.schema.json"), web_get, report, Path("s.yaml"))


def test_scenario_schema_is_v0_6():
    assert "v0.6" in v.load_schema("scenario.schema.json")["description"]


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
                                   "level": "transport", "view": "tcp", "labels": [],
                                   "description": "x"})
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


# --- behavior profiles and distributions (scenario contract v0.3) -----------

@pytest.fixture()
def web_profile() -> dict:
    return v.load_yaml(EXAMPLES / "modules/http1/scenarios-pending/web_profile.yaml")


def test_web_profile_example_is_valid_and_checked(web_profile, http_manifest):
    assert not check_scenario(web_profile, http_manifest).findings


def test_pending_scenarios_are_checked_by_the_default_run():
    assert not v.run([EXAMPLES / "modules"], [], None).errors


def test_undefined_profile_fails(web_profile, http_manifest):
    web_profile["profiles"] = {"firefox": {"Accept-Language": "en"}}
    assert any("undefined profile 'chrome'" in m for m in messages(check_scenario(web_profile, http_manifest)))


def test_profile_values_take_no_line_breaks(web_profile):
    report = v.Report()
    web_profile["profiles"]["chrome"]["Accept-Language"] = "en\r\nX: y"
    assert not v.schema_errors(v.load_schema("scenario.schema.json"), web_profile, report, Path("s.yaml"))


@pytest.mark.parametrize("text, fragment", [
    ("{{ dist.pareto(1.2, 4096) }}", None),
    ("{{ dist.pareto(1, 4096, cap=65536) }}", None),
    ("{{ dist.exponential(mean=20) }}", None),
    ("{{ dist.uniform(10, 20) }}", None),
    ("{{ dist.zipf(1.2) }}", "unknown distribution 'zipf'"),
    ("{{ dist.pareto(1.2) }}", "misses required parameter 'scale'"),
    ("{{ dist.pareto(0, 4096) }}", "'alpha' of 'dist.pareto' must be positive"),
    ("{{ dist.uniform(20, 10) }}", "low greater than high"),
    ("{{ dist.uniform(-1, 10) }}", "'low' of 'dist.uniform' must not be negative"),
    ("{{ dist.pareto('a', 4096) }}", "takes float, got text"),
    ("{{ dist.pareto }}", "call it with parentheses"),
    ("{{ profiles }}", "must have the form profiles.<name>"),
])
def test_dist_calls(text, fragment):
    issues = [i.message for i in v.check_template(text, v.TemplateScope(profiles={"chrome"}))]
    if fragment is None:
        assert issues == []
    else:
        assert any(fragment in m for m in issues)


def test_dist_result_is_int():
    scope = v.TemplateScope()
    assert v.check_template("{{ fill(dist.pareto(1.2, 4096)) }}", scope) == []
    assert v.check_template("{{ dist.pareto(dist.uniform(1, 2), 4096) }}", scope) == []


def test_dist_and_profiles_are_reserved_module_names():
    assert {"dist", "profiles"} <= v.RESERVED_ROOTS


# --- int_bytes and now (scenario contract v0.6, CON-19) ----------------------

@pytest.mark.parametrize("text", [
    "{{ int_bytes(258, 2) }}", "{{ int_bytes(258, 2, 'little') }}", "{{ int_bytes(value=1, width=1, endianness='big') }}",
    "{{ int_bytes(vars.size, 4) }}", "{{ int_bytes(rand_int(0, 65535), 2) }}", "{{ int_bytes(255, 1) }}",
    "{{ int_bytes(0, 8) }}", "{{ int_bytes(9223372036854775807, 8) }}", "{{ int_bytes(vars.size, vars.size) }}",
    "{{ int_bytes(1, 2, vars.order) }}",
    "{{ now('http_date') }}", "{{ now('epoch_s') }}", "{{ now(format='epoch_ms') }}", "{{ now(vars.order) }}",
    "Date: {{ now('http_date') }}\\r\\nX-Time: {{ now('epoch_ms') }}",
])
def test_int_bytes_and_now_accept(text):
    issues = v.check_template(text, {"size", "order"})
    assert [i for i in issues if i.severity == "error"] == []


@pytest.mark.parametrize("text,needle", [
    ("{{ int_bytes(1) }}", "misses required parameter 'width'"),
    ("{{ int_bytes(width=2) }}", "misses required parameter 'value'"),
    ("{{ int_bytes(1, 2, 'big', 4) }}", "at most 3 positional argument"),
    ("{{ int_bytes(1, 0) }}", "width from 1 to 8"),
    ("{{ int_bytes(1, 9) }}", "width from 1 to 8"),
    ("{{ int_bytes(1, 2, 'middle') }}", "'big' or 'little'"),
    ("{{ int_bytes(-1, 2) }}", "does not fit 2 byte(s)"),
    ("{{ int_bytes(256, 1) }}", "does not fit 1 byte(s)"),
    ("{{ int_bytes(65536, 2) }}", "does not fit 2 byte(s)"),
    ("{{ int_bytes(-1, vars.size) }}", "does not fit"),
    ("{{ int_bytes('1', 2) }}", "takes int, got text"),
    ("{{ int_bytes(1, 2.5) }}", "takes int, got float"),
    ("{{ int_bytes(1, 2, 7) }}", "takes text, got int"),
    ("{{ int_bytes(1, 2, bits=3) }}", "has no parameter 'bits'"),
    ("{{ now() }}", "misses required parameter 'format'"),
    ("{{ now('iso') }}", "unknown format 'iso' of now()"),
    ("{{ now('http_date', 1) }}", "at most 1 positional argument"),
    ("{{ now(1) }}", "takes text, got int"),
    ("{{ now(zone='utc') }}", "has no parameter 'zone'"),
])
def test_int_bytes_and_now_reject(text, needle):
    issues = v.check_template(text, {"size"})
    assert any(needle in i.message for i in issues if i.severity == "error"), issues


def test_int_bytes_value_limits_follow_the_width():
    ok = ["{{ int_bytes(255, 1) }}", "{{ int_bytes(65535, 2) }}", "{{ int_bytes(16777215, 3) }}",
          "{{ int_bytes(4294967295, 4) }}", "{{ int_bytes(9223372036854775807, 8) }}"]
    bad = ["{{ int_bytes(256, 1) }}", "{{ int_bytes(65536, 2) }}", "{{ int_bytes(16777216, 3) }}",
           "{{ int_bytes(4294967296, 4) }}", "{{ int_bytes(9223372036854775808, 8) }}"]
    for text in ok:
        assert not v.check_template(text, set()), text
    for text in bad:
        assert any("does not fit" in i.message for i in v.check_template(text, set())), text


@pytest.mark.parametrize("call,result", [
    ("now('http_date')", "text"), ("now('epoch_s')", "int"), ("now(format='epoch_ms')", "int"),
    ("now(vars.order)", "any"), ("int_bytes(1, 2)", "bytes"),
])
def test_result_types_of_the_new_core_functions(call, result):
    node = v.parse_expression(call)
    scope = v.TemplateScope(vars={"order": "text"})
    assert v._check_node(node, scope, []) == result


def test_now_feeds_int_and_text_parameters():
    scope = v.TemplateScope()
    # epoch_s is an int, so it binds fill(n) and int_bytes(value). http_date is text, so it cannot.
    assert v.check_template("{{ fill(now('epoch_s')) }}", scope) == []
    assert v.check_template("{{ int_bytes(now('epoch_s'), 4) }}", scope) == []
    assert any("takes int, got text" in i.message for i in v.check_template("{{ fill(now('http_date')) }}", scope))


def test_new_core_functions_are_in_the_documented_table():
    readme = (v.CONTRACTS / "README.md").read_text()
    assert "| `int_bytes(value, width, endianness)` |" in readme
    assert "| `now(format)` |" in readme
    assert set(v.CORE_FUNCTIONS) == {"fill", "hex", "rand_int", "int_bytes", "now"}


def test_int_bytes_and_now_in_a_scenario(web_get, http_manifest):
    web_get["flows"][0]["client"].insert(1, {"send": "{{ int_bytes(vars.n, 2) }}{{ now('http_date') }}"})
    web_get["vars"]["n"] = 7
    assert not check_scenario(web_get, http_manifest).errors


# --- variables across after (scenario contract v0.6, CON-19) -----------------

def two_flow_scenario(web_get: dict) -> dict:
    """The first flow binds `token` with set, and a second flow, which runs
    after it, sends the token."""
    first = web_get["flows"][0]
    first["client"].insert(1, {"set": {"token": "{{ rand_int(1000, 9999) }}"}})
    second = copy.deepcopy(first)
    second["id"] = "second"
    second["after"] = first["id"]
    second["client"] = [a for a in second["client"] if "set" not in a]
    second["client"].insert(1, {"send": "T={{ vars.token }}"})
    web_get["flows"].append(second)
    return web_get


def test_flow_after_another_reads_its_variables(web_get, http_manifest):
    assert not check_scenario(two_flow_scenario(web_get), http_manifest).errors


def test_flow_after_a_chain_reads_the_first_flows_variables(web_get, http_manifest):
    doc = two_flow_scenario(web_get)
    third = copy.deepcopy(doc["flows"][1])
    third["id"] = "third"
    third["after"] = ["second"]
    doc["flows"].append(third)
    assert not check_scenario(doc, http_manifest).errors


def test_a_variable_that_nothing_binds_is_still_undefined(web_get, http_manifest):
    doc = two_flow_scenario(web_get)
    doc["flows"][1]["client"][1] = {"send": "T={{ vars.nothing }}"}
    assert any("undefined variable 'nothing'" in m for m in messages(check_scenario(doc, http_manifest)))


def test_the_after_rule_is_in_the_readme_and_the_schema():
    readme = (v.CONTRACTS / "README.md").read_text()
    assert "### Variables across `after`" in readme
    flow = v.load_schema("scenario.schema.json")["$defs"]["flow"]["properties"]["after"]
    assert "variable" in flow["description"]


# --- arrival and size distributions (test configuration v0.3, CON-19) --------

ARRIVALS = [
    {"dist": "constant", "ms": 100},
    {"dist": "constant", "ms": 100, "tolerance_ms": 5},
    {"dist": "exponential", "mean_ms": 250},
    {"dist": "uniform", "min_ms": 10, "max_ms": 90},
    {"dist": "lognormal", "mu": -1.2, "sigma": 0.8},
    {"dist": "lognormal", "mu": 4.5, "sigma": 0.8, "unit": "ms"},
    {"dist": "pareto", "alpha": 1.5, "scale_ms": 20},
]
SIZES = [
    {"dist": "constant", "value": 512},
    {"dist": "constant", "value": 512, "tolerance": 16},
    {"dist": "exponential", "mean": 2048},
    {"dist": "uniform", "min": 100, "max": 900},
    {"dist": "lognormal", "mu": 8.1, "sigma": 1.1},
    {"dist": "pareto", "alpha": 1.2, "scale": 4096},
]


def test_test_config_schema_is_v0_3():
    assert "v0.3" in v.load_schema("test-config.schema.json")["description"]


@pytest.mark.parametrize("arrival", ARRIVALS)
def test_arrival_validates_at_profile_and_entry_level(arrival, smoke, index):
    smoke["profile"]["arrival"] = arrival
    smoke["profile"]["mix"][0]["arrival"] = copy.deepcopy(arrival)
    assert not check_test(smoke, index).errors


@pytest.mark.parametrize("size", SIZES)
def test_size_validates_at_profile_and_entry_level(size, smoke, index):
    smoke["profile"]["size"] = {"request": size, "response": copy.deepcopy(size), "both": copy.deepcopy(size)}
    smoke["profile"]["mix"][0]["size"] = {"response": copy.deepcopy(size)}
    assert not check_test(smoke, index).errors


def test_configurations_without_distributions_stay_valid(smoke, index):
    assert "arrival" not in smoke["profile"] and "size" not in smoke["profile"]
    assert not check_test(smoke, index).errors


@pytest.mark.parametrize("arrival,needle", [
    ({"dist": "zipf", "mean_ms": 1}, "is not valid under any of the given schemas"),
    ({"dist": "exponential"}, "is not valid under any"),
    ({"dist": "exponential", "mean_ms": 0}, "is not valid under any"),
    ({"dist": "exponential", "mean_ms": 5, "extra": 1}, "is not valid under any"),
    ({"dist": "exponential", "mean": 5}, "is not valid under any"),
    ({"mean_ms": 5}, "is not valid under any"),
    ({"dist": "constant", "ms": 0}, "is not valid under any"),
    ({"dist": "lognormal", "mu": 1, "sigma": 0}, "is not valid under any"),
    ({"dist": "lognormal", "mu": 1, "sigma": 1, "unit": "h"}, "is not valid under any"),
    ({"dist": "pareto", "alpha": 1, "scale": 3}, "is not valid under any"),
    ({"dist": "uniform", "min_ms": 9, "max_ms": 3}, "min_ms greater than max_ms"),
    ({"dist": "uniform", "min_ms": -1, "max_ms": 3}, "is not valid under any"),
    ({"ms": 5}, "is not valid under any"),
    ("exponential", "is not valid under any"),
])
def test_malformed_arrival_is_rejected(arrival, needle, smoke, index):
    smoke["profile"]["arrival"] = arrival
    found = messages(check_test(smoke, index))
    assert any(needle in m for m in found), found


@pytest.mark.parametrize("size,needle", [
    ({}, "should be non-empty"),
    ({"both": {"dist": "constant", "value": 1}, "extra": {"dist": "constant", "value": 1}},
     "Additional properties are not allowed"),
    ({"request": {"dist": "exponential", "mean_ms": 5}}, "is not valid under any"),
    ({"request": {"dist": "pareto", "alpha": 1, "scale": 0}}, "is not valid under any"),
    ({"response": {"dist": "uniform", "min": 9, "max": 3}}, "min greater than max"),
    ({"request": {"dist": "constant", "value": -4}}, "is not valid under any"),
    ({"request": 4}, "is not valid under any"),
])
def test_malformed_size_is_rejected(size, needle, smoke, index):
    smoke["profile"]["mix"][0]["size"] = size
    found = messages(check_test(smoke, index))
    assert any(needle in m for m in found), found


def test_distributions_are_not_accepted_elsewhere(smoke, index):
    smoke["load"]["arrival"] = {"dist": "exponential", "mean_ms": 5}
    assert messages(check_test(smoke, index))


def test_uniform_bounds_are_reported_at_the_entry_pointer(smoke, index):
    smoke["profile"]["mix"][0]["size"] = {"request": {"dist": "uniform", "min": 9, "max": 3}}
    report = check_test(smoke, index)
    assert any(f.path == "/profile/mix/0/size/request" for f in report.findings), report.findings


# --- application all and property both (metric catalog 0.3.0, CON-19; 0.4.0, CON-20) ---------

def test_catalog_is_0_3_0_with_property_both(catalog_doc):
    assert catalog_doc["version"] == "0.8.0"
    values = catalog_doc["labels"]["property"]["values"]
    assert "both" in values
    # Every value that 0.2.0 had stays, in its place.
    assert values[:5] == ["session_bytes", "request_bytes", "response_bytes", "inter_arrival", "session_duration"]


def test_catalog_documents_the_application_value_all(catalog_doc):
    assert "all" in catalog_doc["labels"]["application"]["description"].split()
    for name in ("realism.ks_statistic", "realism.ks_p_value"):
        metric = next(m for m in catalog_doc["metrics"] if m["name"] == name)
        assert metric["labels"] == ["application", "property"]


def test_ks_criteria_may_select_all_and_both(smoke, index):
    smoke["criteria"].append({"metric": "realism.ks_p_value", "op": ">=", "value": 0.01,
                              "where": {"application": "all", "property": "both"}})
    assert not check_test(smoke, index).errors


def test_the_module_name_all_is_reserved(tmp_path):
    mod = tmp_path / "all"
    shutil.copytree(EXAMPLES / "modules/dns", mod)
    manifest = v.load_yaml(mod / "manifest.yaml")
    manifest["name"] = "all"
    (mod / "manifest.yaml").write_text(yaml.safe_dump(manifest))
    report = v.Report()
    v.check_module_dir(mod, report, v.ModuleIndex())
    assert any("reserved for the application label value" in m for m in messages(report))


def test_no_shipped_module_is_named_all():
    names = {m.name for m in (v.REPO / "modules").iterdir() if m.is_dir()}
    names |= {m.name for m in (EXAMPLES / "modules").iterdir()}
    assert "all" not in names


# --- embedded copies ----------------------------------------------------------

@pytest.mark.parametrize("source,copy_path", [
    ("contracts/schemas/scenario.schema.json", "controller/internal/registry/schema/scenario.schema.json"),
    ("contracts/schemas/test-config.schema.json", "controller/internal/store/schema/test-config.schema.json"),
    ("contracts/metrics/catalog.yaml", "controller/internal/metrics/catalog.yaml"),
])
def test_controller_copies_are_byte_identical(source, copy_path):
    assert (v.REPO / source).read_bytes() == (v.REPO / copy_path).read_bytes(), copy_path


# --- registry rules (CON-30) ----------------------------------------------------

REGISTRY = v.REPO / "controller/internal/registry"


def module_copy(tmp_path: Path, name: str = "dns", edit=None) -> Path:
    """Copy an example module to tmp_path/<name> and return the directory."""
    mod = tmp_path / name
    shutil.copytree(EXAMPLES / "modules/dns", mod)
    manifest = v.load_yaml(mod / "manifest.yaml")
    manifest["name"] = name
    if edit:
        edit(manifest)
    (mod / "manifest.yaml").write_text(yaml.safe_dump(manifest))
    return mod


def module_errors(mod: Path) -> list[str]:
    report = v.Report()
    v.check_module_dir(mod, report, v.ModuleIndex())
    return messages(report)


def test_copy_of_the_example_module_is_valid(tmp_path):
    assert module_errors(module_copy(tmp_path)) == []


@pytest.mark.parametrize("name", sorted(v.REGISTRY_RESERVED_NAMES))
def test_every_registry_reserved_name_is_rejected(tmp_path, name):
    assert any("reserved" in m for m in module_errors(module_copy(tmp_path, name)))


@pytest.mark.parametrize("name", ["profiles", "dist", "all"])
def test_validator_also_rejects_the_names_the_registry_lacks(tmp_path, name):
    assert any("reserved" in m for m in module_errors(module_copy(tmp_path, name)))


def test_reserved_names_equal_the_registry_list():
    source = (REGISTRY / "manifest.go").read_text()
    body = re.search(r"reservedModuleNames = map\[string\]bool\{([^}]*)\}", source).group(1)
    assert set(re.findall(r'"(\w+)"', body)) == set(v.REGISTRY_RESERVED_NAMES)
    assert v.REGISTRY_RESERVED_NAMES <= v.RESERVED_ROOTS


def test_license_lists_equal_the_registry_lists():
    source = (REGISTRY / "license.go").read_text()

    def ids(var: str) -> tuple[str, ...]:
        body = re.search(var + r" = canonical\((.*?)\n\)", source, re.S).group(1)
        return tuple(re.findall(r'"([^"]+)"', body))

    assert ids("compatibleLicenses") == v.COMPATIBLE_LICENSES
    assert ids("incompatibleLicenses") == v.INCOMPATIBLE_LICENSES
    assert ids("licenseExceptions") == v.LICENSE_EXCEPTIONS


# The cases of TestCheckLicense in controller/internal/registry/license_test.go.
LICENSE_CASES = [
    ("GPL-3.0-only", None), ("gpl-3.0-or-later", None), ("MIT", None),
    ("Apache-2.0 WITH LLVM-exception", None), ("MIT OR GPL-2.0-only", None),
    ("(MIT AND BSD-3-Clause) OR Apache-2.0", None), ("LGPL-2.1-or-later AND CC0-1.0", None),
    ("GPL-2.0-only", "not compatible"), ("MIT AND EPL-2.0", "not compatible"),
    ("GPL-2.0-only OR CDDL-1.0", "not compatible"),
    ("Proprietary", "allow list"), ("LicenseRef-acme", "allow list"), ("MIT OR LicenseRef-acme", "allow list"),
    ("GPL-3.0", "allow list"), ("GPL-2.0+", "allow list"),
    ("Apache-2.0 WITH Made-up-exception", "allow list"),
    ("MIT AND", "ends early"), ("(MIT", "closing parenthesis"), ("MIT BSD-3-Clause", "unexpected"),
    ("", "empty"), ("mit or apache-2.0", "unexpected"),
]


@pytest.mark.parametrize("expr,needle", LICENSE_CASES)
def test_license_problem_follows_the_registry(expr, needle):
    problem = v.license_problem(expr)
    if needle is None:
        assert problem is None
    else:
        assert problem and needle in problem, problem


def test_license_cases_equal_the_registry_tests():
    source = (REGISTRY / "license_test.go").read_text()
    body = source.split("func TestCheckLicense", 1)[1].split("for _, tt", 1)[0]
    go = re.findall(r'\{"([^"]*)", (nil|Err\w+)\}', body)
    assert [e for e, _ in go] == [e for e, _ in LICENSE_CASES]
    assert [w == "nil" for _, w in go] == [n is None for _, n in LICENSE_CASES]


@pytest.mark.parametrize("license_text", ["Proprietary", "GPL-2.0-only", "MIT OR"])
def test_module_with_a_refused_license_is_rejected(tmp_path, license_text):
    mod = module_copy(tmp_path, edit=lambda m: m.update(license=license_text))
    assert any(m.startswith("license ") for m in module_errors(mod))


def test_module_with_an_allowed_license_expression_is_valid(tmp_path):
    mod = module_copy(tmp_path, edit=lambda m: m.update(license="Apache-2.0 OR GPL-2.0-only"))
    assert module_errors(mod) == []


@pytest.mark.parametrize("bad", ["notes it.txt", "a+b.txt", "caf\u00e9.txt", "q?.txt"])
def test_file_name_characters_outside_the_relpath_set_are_rejected(tmp_path, bad):
    mod = module_copy(tmp_path)
    (mod / bad).write_text("x")
    assert any("cannot name" in m for m in module_errors(mod))


def test_bad_directory_name_is_rejected_for_files_the_manifest_does_not_name(tmp_path):
    mod = module_copy(tmp_path)
    (mod / "my dir").mkdir()
    (mod / "my dir" / "ok.txt").write_text("x")
    errors = module_errors(mod)
    assert any("cannot name" in m for m in errors)


def test_dotfiles_and_nested_names_are_allowed(tmp_path):
    mod = module_copy(tmp_path)
    (mod / "notes").mkdir()
    (mod / "notes" / ".keep-1_a.txt").write_text("x")
    assert module_errors(mod) == []


@pytest.mark.skipif(not hasattr(os, "symlink"), reason="symbolic links unavailable")
@pytest.mark.parametrize("kind", ["file", "dir", "dangling"])
def test_symbolic_link_in_the_tree_is_rejected(tmp_path, kind):
    mod = module_copy(tmp_path)
    target = {"file": mod / "manifest.yaml", "dir": mod / "scenarios", "dangling": tmp_path / "missing"}[kind]
    (mod / "link").symlink_to(target)
    assert any("symbolic link" in m for m in module_errors(mod))


@pytest.mark.skipif(not hasattr(os, "symlink"), reason="symbolic links unavailable")
def test_a_linked_module_root_is_not_a_symbolic_link_error(tmp_path):
    (tmp_path / "real").mkdir()
    real = module_copy(tmp_path / "real", "dns")
    parent = tmp_path / "parent"
    parent.mkdir()
    (parent / "dns").symlink_to(real)
    assert module_errors(parent / "dns") == []


@pytest.mark.skipif(not hasattr(os, "symlink"), reason="symbolic links unavailable")
def test_manifest_path_that_is_a_link_does_not_exist_for_the_registry(tmp_path):
    mod = module_copy(tmp_path)
    scenario = mod / "scenarios" / "resolve.yaml"
    moved = mod / "resolve-real.yaml"
    scenario.rename(moved)
    scenario.symlink_to(moved)
    errors = module_errors(mod)
    assert any("does not exist" in m for m in errors)


def test_manifest_path_that_is_a_directory_does_not_exist_for_the_registry(tmp_path):
    mod = module_copy(tmp_path, edit=lambda m: m.update(assets=[{"name": "dir", "path": "scenarios"}]))
    assert any("file 'scenarios' does not exist" in m for m in module_errors(mod))


def test_expected_bytes_key_that_names_no_scenario_is_rejected(tmp_path):
    mod = module_copy(tmp_path, edit=lambda m: m["expected_bytes"].update(resolv=10))
    errors = module_errors(mod)
    assert any("expected_bytes names 'resolv'" in m for m in errors), errors


def test_expected_bytes_key_of_a_pending_scenario_is_rejected(tmp_path):
    def edit(m):
        m["expected_bytes"]["later"] = 10
    mod = module_copy(tmp_path, edit=edit)
    (mod / "scenarios-pending").mkdir(exist_ok=True)
    pending = v.load_yaml(mod / "scenarios/resolve.yaml")
    pending["scenario"] = "later"
    (mod / "scenarios-pending" / "later.yaml").write_text(yaml.safe_dump(pending))
    assert any("expected_bytes names 'later'" in m for m in module_errors(mod))


def test_expected_bytes_check_waits_when_a_scenario_file_is_missing(tmp_path):
    mod = module_copy(tmp_path)
    (mod / "scenarios" / "resolve.yaml").unlink()
    errors = module_errors(mod)
    assert any("does not exist" in m for m in errors)
    assert not any("expected_bytes" in m for m in errors)


def test_expected_bytes_of_the_examples_name_scenarios():
    for name in ("dns", "http1"):
        manifest = v.load_yaml(EXAMPLES / "modules" / name / "manifest.yaml")
        assert manifest.get("expected_bytes"), name


# --- the extension.wasm path (CON-30) ---------------------------------------------

@pytest.mark.parametrize("path", ["dns.wasm", "ext/extension.wasm", "manifest.yaml", "extension.wasm.bak"])
def test_extension_wasm_other_than_the_root_file_is_rejected(path, dns_manifest):
    dns_manifest["extension"] = {"wasm": path, "hooks": ["render"]}
    assert manifest_report(dns_manifest).errors


def test_extension_wasm_root_file_is_accepted(dns_manifest):
    dns_manifest["extension"] = {"wasm": "extension.wasm", "hooks": ["render"]}
    assert not manifest_report(dns_manifest).errors


def test_extension_file_must_exist_in_the_module(tmp_path):
    mod = module_copy(tmp_path, edit=lambda m: m.update(extension={"wasm": "extension.wasm", "hooks": ["render"]}))
    assert any("file 'extension.wasm' does not exist" in m for m in module_errors(mod))
    (mod / "extension.wasm").write_bytes(b"\0asm")
    assert module_errors(mod) == []


def test_manifest_schema_matches_the_controller_copy():
    assert (v.SCHEMAS / "module-manifest.schema.json").read_bytes() == (
        REGISTRY / "schema/module-manifest.schema.json").read_bytes()
