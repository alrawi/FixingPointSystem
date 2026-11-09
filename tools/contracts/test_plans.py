# SPDX-License-Identifier: GPL-3.0-only
"""Tests for the agent plan checker. Run with: python3 -m pytest tools/contracts/test_plans.py"""

from __future__ import annotations

import copy
import json
import shutil
import sys
from pathlib import Path

import pytest

pytest.importorskip("google.protobuf")
pytest.importorskip("grpc_tools")

sys.path.insert(0, str(Path(__file__).parent))
import check_plans as cp

EXAMPLE = cp.PLANS / "http_dns_smoke.json"
MODULES = cp.v.EXAMPLES / "modules"


@pytest.fixture()
def plan() -> dict:
    return json.loads(EXAMPLE.read_text(encoding="utf-8"))


def errors_for(doc: dict, tmp_path: Path, modules: Path | None = MODULES) -> list[str]:
    path = tmp_path / "plan.json"
    path.write_text(json.dumps(doc), encoding="utf-8")
    report = cp.v.Report()
    cp.check_plan_file(path, report, modules)
    return [f.message for f in report.errors]


def test_examples_pass() -> None:
    assert cp.main([]) == 0


def test_example_is_canonical(plan: dict) -> None:
    assert cp.canonical(cp.parse_plan(json.dumps(plan))) == plan


def test_unknown_field_fails(plan: dict, tmp_path: Path) -> None:
    plan["workers"][0]["threads"] = 4
    assert any(
        "does not match fps.agent.v1.Plan" in m for m in errors_for(plan, tmp_path)
    )


def test_wrong_enum_fails(plan: dict, tmp_path: Path) -> None:
    plan["topology"] = "TOPOLOGY_MESH"
    assert any(
        "does not match fps.agent.v1.Plan" in m for m in errors_for(plan, tmp_path)
    )


def test_snake_case_names_are_not_canonical(plan: dict, tmp_path: Path) -> None:
    plan["plan_id"] = plan.pop("planId")
    assert any("canonical form" in m for m in errors_for(plan, tmp_path))


def test_default_values_are_not_canonical(plan: dict, tmp_path: Path) -> None:
    plan["workers"][0]["hosts"][0]["offset"] = 0
    assert any("canonical form" in m for m in errors_for(plan, tmp_path))


def test_missing_seed_fails(plan: dict, tmp_path: Path) -> None:
    del plan["seed"]
    assert any("seed is required" in m for m in errors_for(plan, tmp_path))


def test_module_digest_mismatch_fails(plan: dict, tmp_path: Path) -> None:
    mods = tmp_path / "modules"
    shutil.copytree(MODULES, mods)
    (mods / "dns" / "README.md").write_text("changed\n", encoding="utf-8")
    assert any("module dns has digest" in m for m in errors_for(plan, tmp_path, mods))


def test_module_digest_ignores_file_order(tmp_path: Path) -> None:
    a, b = tmp_path / "a", tmp_path / "b"
    for root, order in ((a, ["x", "y/z"]), (b, ["y/z", "x"])):
        for name in order:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(name, encoding="utf-8")
    assert cp.module_tree_digest(a) == cp.module_tree_digest(b)


def test_module_digest_rejects_symlinks(tmp_path: Path) -> None:
    (tmp_path / "real").write_text("x", encoding="utf-8")
    (tmp_path / "link").symlink_to(tmp_path / "real")
    with pytest.raises(ValueError, match="symbolic link"):
        cp.module_tree_digest(tmp_path)


def test_scenario_digest_mismatch_fails(plan: dict, tmp_path: Path) -> None:
    plan["scenarios"][0]["source"] += "# edited\n"
    assert any(
        "digest does not match the source" in m for m in errors_for(plan, tmp_path)
    )


def test_invalid_scenario_source_fails(plan: dict, tmp_path: Path) -> None:
    source = plan["scenarios"][0]["source"].replace("min_bytes: 12", "min_bytes: -1", 1)
    plan["scenarios"][0]["source"] = source
    plan["scenarios"][0]["digest"] = cp.sha256_digest(source.encode())
    assert any("/source" in f for f in _paths(plan, tmp_path))


def _paths(doc: dict, tmp_path: Path) -> list[str]:
    path = tmp_path / "plan.json"
    path.write_text(json.dumps(doc), encoding="utf-8")
    report = cp.v.Report()
    cp.check_plan_file(path, report, MODULES)
    return [f.path for f in report.errors]


def test_unknown_mix_entry_fails(plan: dict, tmp_path: Path) -> None:
    plan["workers"][1]["rates"][0]["mixEntry"] = "video"
    assert any("unknown mix entry 'video'" in m for m in errors_for(plan, tmp_path))


def test_weights_must_sum_to_one(plan: dict, tmp_path: Path) -> None:
    plan["mix"][0]["weight"] = 0.5
    assert any("sum to 1" in m for m in errors_for(plan, tmp_path))


def test_overlapping_host_ranges_fail(plan: dict, tmp_path: Path) -> None:
    plan["workers"][1]["hosts"][0]["offset"] = 100
    assert any("overlaps worker 0" in m for m in errors_for(plan, tmp_path))


def test_host_range_outside_slice_fails(plan: dict, tmp_path: Path) -> None:
    plan["workers"][1]["hosts"][0]["count"] = 200
    assert any("outside the slice" in m for m in errors_for(plan, tmp_path))


def test_pool_larger_than_prefix_fails(plan: dict, tmp_path: Path) -> None:
    plan["hostPools"][0]["poolCount"] = 300
    assert any("usable addresses" in m for m in errors_for(plan, tmp_path))


def test_rate_kind_must_match_objective(plan: dict, tmp_path: Path) -> None:
    plan["load"]["objective"] = "LOAD_OBJECTIVE_CONCURRENT_SESSIONS"
    assert any("needs concurrentSessions" in m for m in errors_for(plan, tmp_path))


def test_hold_phase_needs_equal_fractions(plan: dict, tmp_path: Path) -> None:
    plan["load"]["phases"][1]["endFraction"] = 0.5
    assert any("equal start and end" in m for m in errors_for(plan, tmp_path))


def test_app_sim_needs_both_roles(plan: dict, tmp_path: Path) -> None:
    doc = copy.deepcopy(plan)
    doc["hostPools"] = [p for p in doc["hostPools"] if p["role"] == "HOST_ROLE_CLIENT"]
    assert any("app_sim needs" in m for m in errors_for(doc, tmp_path))


# Agent control protocol 0.2.2 on the CON-06 shape. Agents dial the controller, so the
# controller serves the only service and the agent serves none.

GRPC_CODES = {
    "INVALID_ARGUMENT": 3,
    "NOT_FOUND": 5,
    "RESOURCE_EXHAUSTED": 8,
    "FAILED_PRECONDITION": 9,
    "UNIMPLEMENTED": 12,
    "INTERNAL": 13,
}


def _file() -> object:
    return cp.proto_pool().FindFileByName(cp.AGENT_PROTO)


def test_controller_service_is_the_only_service() -> None:
    assert list(_file().services_by_name) == ["ControllerService"]


def test_register_is_unary_and_session_is_bidirectional() -> None:
    svc = _file().services_by_name["ControllerService"]
    register, session = svc.methods_by_name["Register"], svc.methods_by_name["Session"]
    assert not register.client_streaming and not register.server_streaming
    assert session.client_streaming and session.server_streaming
    assert register.input_type.name == "RegisterRequest"
    assert session.input_type.name == "SessionRequest"
    assert session.output_type.name == "SessionResponse"


def _oneof(message: str, oneof: str) -> dict[str, str]:
    desc = cp.proto_pool().FindMessageTypeByName(f"fps.agent.v1.{message}")
    return {f.name: f.message_type.name for f in desc.oneofs_by_name[oneof].fields}


def test_agent_messages_cover_heartbeats_results_and_counters() -> None:
    assert _oneof("SessionRequest", "body") == {
        "open": "SessionOpen",
        "heartbeat": "Heartbeat",
        "result": "CommandResult",
        "snapshot": "CounterSnapshot",
    }


def test_controller_messages_cover_the_commands() -> None:
    assert _oneof("SessionResponse", "body") == {
        "heartbeat_ack": "HeartbeatAck",
        "prepare": "PrepareCommand",
        "start": "StartCommand",
        "stop": "StopCommand",
        "update_rates": "UpdateRatesCommand",
        "start_capture": "StartCaptureCommand",
        "stop_capture": "StopCaptureCommand",
        "fetch_capture": "FetchCaptureCommand",
    }
    assert _oneof("CommandResult", "result") == {
        "prepare": "PrepareResult",
        "start": "StartResult",
        "stop": "StopResult",
        "update_rates": "UpdateRatesResult",
        "start_capture": "StartCaptureResult",
        "stop_capture": "StopCaptureResult",
        "fetch_capture": "FetchCaptureResult",
    }


def test_upload_capture_is_a_client_streaming_call_with_ordered_chunks() -> None:
    svc = _file().services_by_name["ControllerService"]
    assert list(svc.methods_by_name) == ["Register", "Session", "UploadCapture"]
    upload = svc.methods_by_name["UploadCapture"]
    assert upload.client_streaming and not upload.server_streaming
    assert upload.input_type.name == "UploadCaptureRequest"
    assert upload.output_type.name == "UploadCaptureResponse"
    assert _oneof("UploadCaptureRequest", "body") == {
        "open": "UploadCaptureOpen",
        "chunk": "CaptureChunk",
        "end": "CaptureEnd",
    }
    assert _numbers("CaptureChunk") == {"offset": 1, "data": 2}


def _numbers(message: str) -> dict[str, int]:
    desc = cp.proto_pool().FindMessageTypeByName(f"fps.agent.v1.{message}")
    return {f.name: f.number for f in desc.fields}


def test_version_0_1_field_numbers_are_unchanged() -> None:
    """Versions 0.2.0, 0.2.1, and 0.2.2 only add fields, so every 0.1.0 number keeps its meaning."""
    assert {k: v for k, v in _numbers("ModuleRef").items() if v <= 5} == {
        "name": 1,
        "version": 2,
        "digest": 3,
        "signature": 4,
        "signature_key_id": 5,
    }
    assert {k: v for k, v in _numbers("SessionResponse").items() if v <= 5} == {
        "command_id": 1,
        "heartbeat_ack": 2,
        "prepare": 3,
        "start": 4,
        "stop": 5,
    }


def test_module_ref_carries_settings_as_field_6() -> None:
    desc = cp.proto_pool().FindMessageTypeByName("fps.agent.v1.ModuleRef")
    field = desc.fields_by_name["settings"]
    assert field.number == 6
    assert field.message_type.full_name == "google.protobuf.Struct"


def test_plan_with_module_settings_parses_and_round_trips(plan: dict, tmp_path: Path) -> None:
    settings = {"zone_file": "zones/example.zone", "ttl": 300, "names": ["a", "b"]}
    plan["modules"][0]["settings"] = settings
    assert errors_for(plan, tmp_path) == []
    assert cp.canonical(cp.parse_plan(json.dumps(plan))) == plan


def test_error_codes_equal_grpc_status_codes() -> None:
    enum = cp.proto_pool().FindEnumTypeByName("fps.agent.v1.ErrorCode")
    values = {v.name: v.number for v in enum.values}
    assert values.pop("ERROR_CODE_UNSPECIFIED") == 0
    assert values == {f"ERROR_CODE_{k}": n for k, n in GRPC_CODES.items()}


def test_readme_lists_every_error_code() -> None:
    readme = (cp.v.CONTRACTS / "README.md").read_text(encoding="utf-8")
    for name in GRPC_CODES:
        assert f"`ERROR_CODE_{name}`" in readme
