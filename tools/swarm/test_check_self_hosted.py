# SPDX-License-Identifier: GPL-3.0-only
"""Tests for tools/swarm/check_self_hosted.py. No test reaches the network."""

from __future__ import annotations

import copy
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

sys.path.insert(0, str(Path(__file__).parent))
import check_self_hosted as chk

ROOT = Path(__file__).resolve().parents[2]
REAL = ROOT / ".github" / "workflows"

GOOD: dict[str, Any] = {
    "name": "nightly",
    "on": {"schedule": [{"cron": "17 2 * * *"}], "workflow_dispatch": None},
    "permissions": {"contents": "read"},
    "jobs": {
        "bench": {
            "if": "github.ref == 'refs/heads/main'",
            "runs-on": ["self-hosted", "linux", "fps-bench"],
            "timeout-minutes": 45,
            "steps": [
                {"uses": "actions/checkout@v4", "with": {"persist-credentials": False}},
                {"run": "make bench-s1"},
            ],
        }
    },
}


def variant(**changes: Any) -> dict[str, Any]:
    """A copy of GOOD with top level keys replaced, or removed when None."""
    wf = copy.deepcopy(GOOD)
    for key, value in changes.items():
        if value is None:
            wf.pop(key, None)
        else:
            wf[key] = value
    return wf


def job_variant(**changes: Any) -> dict[str, Any]:
    wf = copy.deepcopy(GOOD)
    for key, value in changes.items():
        if value is None:
            wf["jobs"]["bench"].pop(key, None)
        else:
            wf["jobs"]["bench"][key] = value
    return wf


def run(
    tmp_path: Path, workflow: dict[str, Any], name: str = "w.yml"
) -> list[chk.Finding]:
    return run_all(tmp_path, {name: workflow})[0]


def run_all(
    tmp_path: Path, files: dict[str, dict[str, Any]]
) -> tuple[list[chk.Finding], list[str]]:
    for name, workflow in files.items():
        (tmp_path / name).write_text(yaml.safe_dump(workflow, sort_keys=False))
    return chk.check_directory(tmp_path)


def rules(findings: list[chk.Finding]) -> set[str]:
    return {f.rule for f in findings}


def test_good_workflow_passes(tmp_path: Path) -> None:
    findings, warnings = run_all(tmp_path, {"w.yml": GOOD})
    assert findings == []
    assert warnings == []


@pytest.mark.parametrize(
    "event",
    [
        "push",
        "pull_request",
        "pull_request_target",
        "issue_comment",
        "workflow_run",
        "workflow_call",
        "issues",
        "pull_request_review_comment",
        "release",
        "repository_dispatch",
    ],
)
def test_other_trigger_fails_in_a_mapping(tmp_path: Path, event: str) -> None:
    on = dict(GOOD["on"])
    on[event] = {"branches": ["main"]} if event == "push" else None
    findings = run(tmp_path, variant(on=on))
    assert rules(findings) == {"trigger"}
    assert event in findings[0].message


@pytest.mark.parametrize("event", ["push", "pull_request", "workflow_call"])
def test_other_trigger_fails_as_a_list_or_a_name(tmp_path: Path, event: str) -> None:
    assert rules(run(tmp_path, variant(on=[event, "schedule"]))) == {"trigger"}
    assert rules(run(tmp_path, variant(on=event))) == {"trigger"}


def test_dispatch_alone_and_schedule_alone_pass(tmp_path: Path) -> None:
    assert run(tmp_path, variant(on="workflow_dispatch")) == []
    assert run(tmp_path, variant(on=["schedule"])) == []


def test_on_key_read_as_true_by_yaml_1_1(tmp_path: Path) -> None:
    text = (
        "on:\n  push:\n  workflow_dispatch:\npermissions:\n  contents: read\n"
        "jobs:\n  j:\n    runs-on: fps-bench\n    timeout-minutes: 5\n    steps:\n"
        "      - uses: actions/checkout@v4\n        with:\n          persist-credentials: false\n"
    )
    assert True in yaml.safe_load(text)  # PyYAML maps `on` to True
    (tmp_path / "w.yml").write_text(text)
    findings, _ = chk.check_directory(tmp_path)
    assert rules(findings) == {"trigger"}


def test_workflow_without_triggers_fails(tmp_path: Path) -> None:
    assert rules(run(tmp_path, variant(on=None))) == {"trigger"}


def test_job_if_guard_does_not_excuse_a_push_trigger(tmp_path: Path) -> None:
    # The shape of qa-bench.yml: push and pull_request, and a guarded nightly job.
    on = dict(GOOD["on"])
    on["push"] = {"branches": ["main"]}
    on["pull_request"] = None
    job = copy.deepcopy(GOOD["jobs"]["bench"])
    job["if"] = (
        "github.event_name == 'schedule' || "
        "(github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main')"
    )
    findings = run(tmp_path, variant(on=on, jobs={"bench": job}))
    assert rules(findings) == {"trigger"}
    assert "pull_request" in findings[0].message and "push" in findings[0].message


@pytest.mark.parametrize(
    "runs_on",
    [
        "self-hosted",
        "fps-bench",
        ["self-hosted", "linux", "fps-bench"],
        ["linux", "x64"],
        "my-runner",
        ["ubuntu-24.04", "fps-bench"],
        {"group": "bench", "labels": ["fps-bench"]},
        "${{ inputs.runner }}",
        "${{ matrix.os }}",
        "",
    ],
)
def test_every_non_hosted_label_counts_as_self_hosted(
    tmp_path: Path, runs_on: Any
) -> None:
    on = {"push": None, "schedule": [{"cron": "1 2 * * *"}]}
    wf = job_variant(**{"runs-on": runs_on})
    wf["on"] = on
    assert rules(run(tmp_path, wf)) == {"trigger"}


@pytest.mark.parametrize(
    "runs_on",
    ["ubuntu-24.04", "ubuntu-latest", "windows-2022", "macos-14", ["ubuntu-22.04-arm"]],
)
def test_hosted_labels_are_not_checked(tmp_path: Path, runs_on: Any) -> None:
    wf = job_variant(**{"runs-on": runs_on, "steps": [{"run": "true"}]})
    wf["on"] = ["push", "pull_request", "workflow_call"]
    assert run(tmp_path, wf) == []


def test_matrix_of_hosted_labels_passes_and_a_mixed_matrix_fails(
    tmp_path: Path,
) -> None:
    job = {
        "runs-on": "${{ matrix.os }}",
        "strategy": {"matrix": {"os": ["ubuntu-24.04", "macos-14"]}},
        "steps": [{"run": "true"}],
    }
    base = variant(on=["push"], jobs={"j": job})
    assert run(tmp_path, base) == []
    mixed = copy.deepcopy(base)
    mixed["jobs"]["j"]["strategy"]["matrix"]["os"].append("fps-bench")
    assert "trigger" in rules(run(tmp_path, mixed))
    included = copy.deepcopy(base)
    included["jobs"]["j"]["strategy"]["matrix"]["include"] = [{"os": "fps-bench"}]
    assert "trigger" in rules(run(tmp_path, included))


def test_job_that_calls_a_reusable_workflow_has_no_runner(tmp_path: Path) -> None:
    wf = variant(on=["push"], jobs={"call": {"uses": "./.github/workflows/x.yml"}})
    assert run(tmp_path, wf) == []


@pytest.mark.parametrize(
    "permissions",
    [
        None,
        {},
        {"contents": "write"},
        {"contents": "none"},
        {"issues": "read"},
        {"contents": "read", "pull-requests": "write"},
        "read-all",
        "write-all",
    ],
)
def test_permissions_must_be_contents_read_and_no_write(
    tmp_path: Path, permissions: Any
) -> None:
    assert rules(run(tmp_path, variant(permissions=permissions))) == {"permissions"}


def test_permissions_may_sit_on_the_job_instead_of_the_workflow(tmp_path: Path) -> None:
    wf = variant(permissions=None)
    wf["jobs"]["bench"]["permissions"] = {"contents": "read"}
    assert run(tmp_path, wf) == []


def test_job_permissions_override_the_workflow(tmp_path: Path) -> None:
    wf = job_variant(permissions={"contents": "write"})
    findings = run(tmp_path, wf)
    assert rules(findings) == {"permissions"}
    assert "job" in findings[0].message
    # and a stricter job does not inherit a broad workflow
    wf = variant(permissions={"contents": "write"})
    wf["jobs"]["bench"]["permissions"] = {"contents": "read"}
    assert run(tmp_path, wf) == []


def test_extra_read_permission_is_allowed(tmp_path: Path) -> None:
    assert (
        run(tmp_path, variant(permissions={"contents": "read", "actions": "read"}))
        == []
    )


@pytest.mark.parametrize("value", [False, "false", "False"])
def test_persist_credentials_false_spellings_pass(tmp_path: Path, value: Any) -> None:
    steps = [{"uses": "actions/checkout@v4", "with": {"persist-credentials": value}}]
    assert run(tmp_path, job_variant(steps=steps)) == []


@pytest.mark.parametrize(
    "step",
    [
        {"uses": "actions/checkout@v4"},
        {"uses": "actions/checkout@v4", "with": {"fetch-depth": 0}},
        {"uses": "actions/checkout@v4", "with": {"persist-credentials": True}},
        {
            "uses": "actions/checkout@v4",
            "with": {"persist-credentials": "${{ inputs.p }}"},
        },
        {"uses": "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683"},
    ],
)
def test_checkout_without_persist_credentials_false_fails(
    tmp_path: Path, step: dict[str, Any]
) -> None:
    assert rules(run(tmp_path, job_variant(steps=[step]))) == {"checkout"}


def test_every_checkout_step_needs_the_setting(tmp_path: Path) -> None:
    steps = [
        {"uses": "actions/checkout@v4", "with": {"persist-credentials": False}},
        {"uses": "actions/checkout@v4", "with": {"path": "other"}},
    ]
    findings = run(tmp_path, job_variant(steps=steps))
    assert rules(findings) == {"checkout"} and len(findings) == 1


def test_job_without_checkout_fails(tmp_path: Path) -> None:
    assert rules(run(tmp_path, job_variant(steps=[{"run": "true"}]))) == {"checkout"}
    assert rules(run(tmp_path, job_variant(steps=None))) == {"checkout"}


def test_other_actions_do_not_count_as_checkout(tmp_path: Path) -> None:
    steps = [
        {"uses": "actions/checkout-fake@v1", "with": {"persist-credentials": False}}
    ]
    assert rules(run(tmp_path, job_variant(steps=steps))) == {"checkout"}


@pytest.mark.parametrize("value", [None, 0, -5, "45", "${{ inputs.t }}", 1.5, True])
def test_timeout_minutes_must_be_a_positive_whole_number(
    tmp_path: Path, value: Any
) -> None:
    assert rules(run(tmp_path, job_variant(**{"timeout-minutes": value}))) == {
        "timeout"
    }


def test_a_timeout_on_the_workflow_does_not_exist_and_is_not_accepted(
    tmp_path: Path,
) -> None:
    wf = job_variant(**{"timeout-minutes": None})
    wf["timeout-minutes"] = 10
    assert rules(run(tmp_path, wf)) == {"timeout"}


def test_all_rules_report_together_and_name_file_and_job(tmp_path: Path) -> None:
    wf = job_variant(
        steps=[{"uses": "actions/checkout@v4"}], **{"timeout-minutes": None}
    )
    wf["on"] = ["push"]
    wf["permissions"] = "write-all"
    findings = run(tmp_path, wf, name="bad.yml")
    assert rules(findings) == {"trigger", "permissions", "checkout", "timeout"}
    assert any(str(f).startswith("bad.yml:bench: checkout:") for f in findings)
    assert any(str(f).startswith("bad.yml: trigger:") for f in findings)


def test_missing_main_guard_warns_without_failing(tmp_path: Path) -> None:
    findings, warnings = run_all(tmp_path, {"w.yml": job_variant(**{"if": None})})
    assert findings == []
    assert len(warnings) == 1 and "refs/heads/main" in warnings[0]
    findings, warnings = run_all(tmp_path, {"w.yml": job_variant(**{"if": "always()"})})
    assert findings == [] and len(warnings) == 1


def test_only_the_self_hosted_job_is_held_to_the_rules(tmp_path: Path) -> None:
    wf = copy.deepcopy(GOOD)
    wf["jobs"]["lint"] = {"runs-on": "ubuntu-24.04", "steps": [{"run": "true"}]}
    assert run(tmp_path, wf) == []
    wf["jobs"]["lint"]["timeout-minutes"] = None
    assert run(tmp_path, wf) == []


def test_ignores_files_that_are_not_workflows(tmp_path: Path) -> None:
    (tmp_path / "README.md").write_text("on: push\n")
    assert run(tmp_path, GOOD, name="a.yaml") == []


def test_invalid_yaml_is_an_error(tmp_path: Path) -> None:
    (tmp_path / "w.yml").write_text("jobs: [unclosed\n")
    with pytest.raises(chk.WorkflowError):
        chk.check_directory(tmp_path)
    assert chk.main(["--workflows", str(tmp_path)]) == 2


def test_non_mapping_workflow_is_an_error(tmp_path: Path) -> None:
    (tmp_path / "w.yml").write_text("- a\n- b\n")
    assert chk.main(["--workflows", str(tmp_path)]) == 2


def test_main_exit_codes_and_output(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    (tmp_path / "ok.yml").write_text(yaml.safe_dump(GOOD, sort_keys=False))
    assert chk.main(["--workflows", str(tmp_path)]) == 0
    assert "keeps self hosted jobs off untrusted triggers" in capsys.readouterr().out
    (tmp_path / "bad.yml").write_text(
        yaml.safe_dump(variant(on=["push"]), sort_keys=False)
    )
    assert chk.main(["--workflows", str(tmp_path)]) == 1
    assert "bad.yml: trigger:" in capsys.readouterr().out
    assert chk.main(["--workflows", str(tmp_path / "missing")]) == 2


def test_the_good_fixture_is_a_workflow_that_the_check_reads_as_self_hosted() -> None:
    # Guards the tests above: a GOOD that stopped being self hosted would pass everything.
    assert chk.is_self_hosted(GOOD["jobs"]["bench"])
    assert chk.triggers_of(GOOD) == {"schedule", "workflow_dispatch"}


# The real workflows of the repository.


def test_real_workflows_break_no_rule_except_the_known_qa_bench_trigger_gap() -> None:
    """qa-bench.yml names fps-bench in a workflow that push and pull_request start.

    Operations.md rule 1 allows only workflow_dispatch and schedule, so the nightly
    job must move to a workflow of its own (see the Shared page notes of OPS-08).
    Every other workflow passes. This test keeps passing after that move.
    """
    findings, _ = chk.check_directory(REAL)
    assert {(f.file, f.rule) for f in findings} <= {("qa-bench.yml", "trigger")}


def test_qa_bench_nightly_job_alone_would_pass_in_a_workflow_of_its_own(
    tmp_path: Path,
) -> None:
    """The fix that the coordinator applies: same job, only the allowed triggers."""
    real = yaml.safe_load((REAL / "qa-bench.yml").read_text())
    if chk.triggers_of(real) <= chk.ALLOWED_TRIGGERS:
        pytest.skip("qa-bench.yml was already split")
    split = {
        "name": "qa-bench-nightly",
        "on": {k: v for k, v in real[True].items() if k in chk.ALLOWED_TRIGGERS},
        "permissions": real["permissions"],
        "jobs": {"nightly": real["jobs"]["nightly"]},
    }
    findings, warnings = run_all(tmp_path, {"qa-bench-nightly.yml": split})
    assert findings == []
    assert warnings == []
    # and the remainder, with the job removed, is a plain hosted workflow
    rest = copy.deepcopy(real)
    del rest["jobs"]["nightly"]
    findings, _ = run_all(tmp_path, {"qa-bench.yml": rest})
    assert findings == []


def test_real_workflows_all_load() -> None:
    names = [p.name for p in REAL.iterdir() if p.suffix in {".yml", ".yaml"}]
    assert len(names) > 10
    for p in REAL.iterdir():
        if p.suffix in {".yml", ".yaml"}:
            chk.load_workflow(p)
