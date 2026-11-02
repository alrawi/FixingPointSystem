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
    assert not report.errors
    return idx


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
    ("{{ fill(1, 2) }}", "takes 1 argument"),
    ("{{ hex('abc') }}", "even number"),
    ("{{ session.secret }}", "unknown session field"),
    ("{{ other.x }}", "unknown reference root"),
    ("{{ }}", "empty placeholder"),
    ("{{ vars.host ", "unbalanced"),
    ("{{ 1 + 2 }}", "cannot parse"),
])
def test_template_rejects(text, needle):
    issues = v.check_template(text, {"host"})
    assert any(needle in i.message for i in issues if i.severity == "error"), issues


def test_module_helper_is_a_warning():
    issues = v.check_template("{{ http1.request('GET', '/') }}", set())
    assert [i.severity for i in issues] == ["warning"]


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
