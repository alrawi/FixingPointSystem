# SPDX-License-Identifier: GPL-3.0-only
"""Tests for tools/swarm/project_sync.py. No test reaches the network.

The fake runner answers each GraphQL operation by name from the response files
under testdata/project_sync/, which follow the shape of GitHub GraphQL replies.
"""

from __future__ import annotations

import io
import json
import re
import sys
import urllib.request
from pathlib import Path
from typing import Any

import pytest

sys.path.insert(0, str(Path(__file__).parent))
import project_sync as ps

DATA = Path(__file__).parent / "testdata" / "project_sync"
ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "project-sync.yml"
ENV = {"FPS_PROJECT_TOKEN": "test-token", "FPS_REPO": "alrawi/FixingPointSystem"}
PROJECT = "PVT_kwHOAAlrawi4AAA1"


def load(name: str) -> dict[str, Any]:
    data: dict[str, Any] = json.loads((DATA / name).read_text())
    return data


class FakeGraphQL:
    """Replays a response per operation name and records every call."""

    def __init__(self, **responses: str | list[str]) -> None:
        self.responses = {
            k: list(v) if isinstance(v, list) else [v] for k, v in responses.items()
        }
        self.calls: list[tuple[str, dict[str, Any]]] = []

    def __call__(self, query: str, variables: dict[str, Any]) -> dict[str, Any]:
        match = re.match(r"\s*(?:query|mutation) (\w+)", query)
        assert match, query
        name = match.group(1)
        self.calls.append((name, variables))
        queue = self.responses.get(name)
        assert queue, f"no recorded response for {name}"
        return load(queue.pop(0) if len(queue) > 1 else queue[0])

    def names(self) -> list[str]:
        return [n for n, _ in self.calls]

    def mutations(self, name: str) -> list[dict[str, Any]]:
        return [v for n, v in self.calls if n == name]


@pytest.fixture(autouse=True)
def no_network(monkeypatch: pytest.MonkeyPatch) -> None:
    def refuse(*args: object, **kwargs: object) -> None:
        raise AssertionError("test tried to reach the network")

    monkeypatch.setattr(urllib.request, "urlopen", refuse)


def full_repo() -> FakeGraphQL:
    return FakeGraphQL(
        LinkedProjects="linked_projects.json",
        AllIssues=["issues_page1.json", "issues_page2.json"],
        AddItem="add_item.json",
        SetStatus="set_status.json",
    )


def test_without_the_token_the_sync_succeeds_and_does_nothing(
    capsys: pytest.CaptureFixture[str],
) -> None:
    fake = FakeGraphQL()
    assert ps.main([], env={"FPS_PROJECT_TOKEN": ""}, runner=fake) == 0
    assert ps.main([], env={}, runner=fake) == 0
    assert fake.calls == []
    assert "FPS_PROJECT_TOKEN is not set" in capsys.readouterr().out


def test_without_the_token_the_real_runner_is_never_built() -> None:
    assert ps.main(["--issue", "13"], env={}) == 0


def test_label_map_covers_every_status_label_of_the_protocol() -> None:
    import fps_swarm

    assert set(ps.STATUS_OPTIONS) == set(fps_swarm.STATUS_LABELS)
    assert list(ps.STATUS_OPTIONS.values()) == [
        "Backlog",
        "Ready",
        "In progress",
        "Blocked",
        "Stale",
        "Review",
        "Done",
    ]


def test_finds_the_one_project_of_the_owner_linked_to_the_repository() -> None:
    fake = FakeGraphQL(LinkedProjects="linked_projects.json")
    project = ps.find_project(fake, "alrawi/FixingPointSystem", "alrawi", None)
    assert project.id == PROJECT
    assert project.number == 1
    assert project.options["in progress"] == "47fc9ee4"
    assert fake.calls == [
        ("LinkedProjects", {"owner": "alrawi", "name": "FixingPointSystem"})
    ]


def test_two_linked_projects_ask_for_the_project_number() -> None:
    fake = FakeGraphQL(LinkedProjects="linked_two.json")
    with pytest.raises(ps.SyncError, match="FPS_PROJECT_NUMBER"):
        ps.find_project(fake, "alrawi/FixingPointSystem", "alrawi", None)


def test_no_linked_project_is_a_configuration_error(
    capsys: pytest.CaptureFixture[str],
) -> None:
    fake = FakeGraphQL(LinkedProjects="linked_none.json")
    assert ps.main([], env=ENV, runner=fake) == 2
    assert "no project of alrawi is linked" in capsys.readouterr().err


def test_project_number_variable_selects_the_user_project() -> None:
    fake = FakeGraphQL(NumberedProject="numbered_project.json")
    env = dict(ENV, FPS_PROJECT_NUMBER="2")
    project = ps.find_project(fake, env["FPS_REPO"], "alrawi", 2)
    assert project.id == "PVT_kwHOAAlrawi4AAA2"
    assert fake.calls == [("NumberedProject", {"login": "alrawi", "number": 2})]


def test_an_empty_project_number_variable_means_discovery() -> None:
    assert ps.parse_number("", "FPS_PROJECT_NUMBER") is None
    with pytest.raises(ps.SyncError):
        ps.parse_number("one", "FPS_PROJECT_NUMBER")


def test_missing_project_number_reports_the_graphql_error(
    capsys: pytest.CaptureFixture[str],
) -> None:
    fake = FakeGraphQL(NumberedProject="numbered_missing.json")
    env = dict(ENV, FPS_PROJECT_NUMBER="9")
    assert ps.main([], env=env, runner=fake) == 2
    assert "Could not resolve to a ProjectV2" in capsys.readouterr().err


def test_project_without_a_status_field_is_a_configuration_error() -> None:
    fake = FakeGraphQL(LinkedProjects="no_status_field.json")
    with pytest.raises(ps.SyncError, match="no single select field named Status"):
        ps.find_project(fake, "alrawi/FixingPointSystem", "alrawi", None)


def test_token_without_project_scope_fails_loudly(
    capsys: pytest.CaptureFixture[str],
) -> None:
    fake = FakeGraphQL(LinkedProjects="insufficient_scope.json")
    assert ps.main([], env=ENV, runner=fake) == 2
    assert "required scopes" in capsys.readouterr().err


def test_full_sync_sets_changed_items_adds_missing_ones_and_pages(
    capsys: pytest.CaptureFixture[str],
) -> None:
    fake = full_repo()
    assert ps.main([], env=ENV, runner=fake) == 1  # #15 has two status labels
    out = capsys.readouterr().out
    pages = fake.mutations("AllIssues")
    assert [p["after"] for p in pages] == [None, "Y3Vyc29yOjUw"]
    assert fake.mutations("AddItem") == [
        {"project": PROJECT, "content": "I_kwDOFPS0013"}
    ]
    assert fake.mutations("SetStatus") == [
        {
            "project": PROJECT,
            "item": "PVTI_12",
            "field": ps_field(),
            "option": "47fc9ee4",
        },
        {
            "project": PROJECT,
            "item": "PVTI_new13",
            "field": ps_field(),
            "option": "61e4505c",
        },
        {
            "project": PROJECT,
            "item": "PVTI_16",
            "field": ps_field(),
            "option": "1b2c3d4e",
        },
    ]
    assert "#11" not in out
    assert "#12: set Status to In progress" in out
    assert "#13: set Status to Ready, added to the project" in out
    assert "#15: skipped, more than one status label" in out
    assert "done: 3 set, 1 unchanged, 1 skipped, 1 failed" in out


def ps_field() -> str:
    return "PVTSSF_lAHOAAlrawi4AAA1zgStatus"


def test_dry_run_prints_the_plan_and_changes_nothing(
    capsys: pytest.CaptureFixture[str],
) -> None:
    fake = full_repo()
    ps.main(["--dry-run"], env=ENV, runner=fake)
    assert fake.mutations("AddItem") == []
    assert fake.mutations("SetStatus") == []
    assert (
        "#13: would set Status to Ready, added to the project"
        in capsys.readouterr().out
    )


def test_label_event_syncs_only_the_issue_of_the_event(tmp_path: Path) -> None:
    event = tmp_path / "event.json"
    event.write_text(
        json.dumps(
            {"action": "labeled", "issue": {"number": 13}, "label": {"name": "x"}}
        )
    )
    fake = FakeGraphQL(
        LinkedProjects="linked_projects.json",
        OneIssue="issue_13.json",
        SetStatus="set_status.json",
    )
    assert ps.main(["--event", str(event)], env=ENV, runner=fake) == 0
    assert fake.names() == ["LinkedProjects", "OneIssue", "SetStatus"]
    assert fake.mutations("OneIssue")[0]["number"] == 13
    assert fake.mutations("SetStatus")[0]["option"] == "47fc9ee4"


def test_event_without_an_issue_does_nothing(tmp_path: Path) -> None:
    event = tmp_path / "event.json"
    event.write_text(json.dumps({"schedule": "5,35 * * * *"}))
    fake = FakeGraphQL()
    assert ps.main(["--event", str(event)], env=ENV, runner=fake) == 0
    assert fake.calls == []
    pr = tmp_path / "pr.json"
    pr.write_text(json.dumps({"issue": {"number": 80, "pull_request": {}}}))
    assert ps.event_issue_number(pr) is None


def test_matching_status_sends_no_mutation() -> None:
    project = ps.find_project(
        FakeGraphQL(LinkedProjects="linked_projects.json"),
        "alrawi/FixingPointSystem",
        "alrawi",
        None,
    )
    issue = load("issues_page1.json")["data"]["repository"]["issues"]["nodes"][0]
    fake = FakeGraphQL()
    assert ps.sync_issue(fake, project, issue, False, io.StringIO()) == "unchanged"
    assert fake.calls == []


def test_missing_option_on_the_board_is_reported() -> None:
    project = ps.find_project(
        FakeGraphQL(LinkedProjects="linked_projects.json"),
        "alrawi/FixingPointSystem",
        "alrawi",
        None,
    )
    trimmed = ps.Project(
        project.id,
        project.number,
        project.title,
        project.field_id,
        {k: v for k, v in project.options.items() if k != "stale"},
    )
    issue = load("issues_page2.json")["data"]["repository"]["issues"]["nodes"][2]
    out = io.StringIO()
    assert ps.sync_issue(FakeGraphQL(), trimmed, issue, False, out) == "failed"
    assert "no Status option named 'Stale'" in out.getvalue()


def test_workflow_runs_on_label_events_and_on_a_schedule_with_the_secret() -> None:
    text = WORKFLOW.read_text()
    for needle in (
        "issues:",
        "labeled",
        "unlabeled",
        "schedule:",
        "workflow_dispatch:",
        "secrets.FPS_PROJECT_TOKEN",
        "vars.FPS_PROJECT_NUMBER",
        "tools/swarm/project_sync.py",
        "--event",
    ):
        assert needle in text, needle
    cron = re.search(r'cron: "([^"]+)"', text)
    assert cron, "the workflow needs a schedule"
    scan = (ROOT / ".github" / "workflows" / "swarm-scan.yml").read_text()
    assert cron.group(1) not in scan, "run after the scanner, not at the same minute"
    assert "github.token" not in text, (
        "the repository token cannot write a user project"
    )
