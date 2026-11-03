# SPDX-License-Identifier: GPL-3.0-only
"""Mirror the status label of each work item into the Status field of the Project.

The owner's user project groups its Board views by the single select field
Status, and labels change through the REST API, so nothing moves the cards by
itself. This command reads the status label of each issue and sets the Status
option with the same name, adding the issue to the project when it is missing.

It talks to the GraphQL API with the token in FPS_PROJECT_TOKEN. Without that
token it prints a notice and exits with 0, so the workflow stays green until the
owner creates the secret. Tests pass a fake as the `Runner` and never reach the
network.

Exit codes are 0 on success or when the token is absent, 1 when an issue could
not be synced, and 2 on a usage, configuration, or API error.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

GRAPHQL_URL = "https://api.github.com/graphql"
STATUS_FIELD = "Status"

# Each status label and the Status option name it maps to.
STATUS_OPTIONS = {
    "status:backlog": "Backlog",
    "status:ready": "Ready",
    "status:in-progress": "In progress",
    "status:blocked": "Blocked",
    "status:stale": "Stale",
    "status:review": "Review",
    "status:done": "Done",
}

Runner = Callable[[str, dict[str, Any]], dict[str, Any]]

PROJECT_FIELDS = """
  id
  number
  title
  owner { ... on User { login } ... on Organization { login } }
  field(name: "Status") {
    ... on ProjectV2SingleSelectField { id name options { id name } }
  }
"""

ISSUE_FIELDS = """
  id
  number
  labels(first: 50) { nodes { name } }
  projectItems(first: 20) {
    nodes {
      id
      project { id }
      fieldValueByName(name: "Status") {
        ... on ProjectV2ItemFieldSingleSelectValue { optionId name }
      }
    }
  }
"""

Q_LINKED = (
    "query LinkedProjects($owner: String!, $name: String!) {"
    " repository(owner: $owner, name: $name) {"
    " projectsV2(first: 20) { nodes {" + PROJECT_FIELDS + "} } } }"
)
Q_NUMBERED = (
    "query NumberedProject($login: String!, $number: Int!) {"
    " user(login: $login) { projectV2(number: $number) {" + PROJECT_FIELDS + "} } }"
)
Q_ISSUE = (
    "query OneIssue($owner: String!, $name: String!, $number: Int!) {"
    " repository(owner: $owner, name: $name) { issue(number: $number) {"
    + ISSUE_FIELDS
    + "} } }"
)
Q_ISSUES = (
    "query AllIssues($owner: String!, $name: String!, $after: String) {"
    " repository(owner: $owner, name: $name) {"
    " issues(first: 50, after: $after, states: [OPEN, CLOSED]) {"
    " pageInfo { hasNextPage endCursor } nodes {" + ISSUE_FIELDS + "} } } }"
)
M_ADD = (
    "mutation AddItem($project: ID!, $content: ID!) {"
    " addProjectV2ItemById(input: {projectId: $project, contentId: $content})"
    " { item { id } } }"
)
M_SET = (
    "mutation SetStatus($project: ID!, $item: ID!, $field: ID!, $option: String!) {"
    " updateProjectV2ItemFieldValue(input: {projectId: $project, itemId: $item,"
    " fieldId: $field, value: {singleSelectOptionId: $option}})"
    " { projectV2Item { id } } }"
)


class SyncError(Exception):
    """A configuration or API error that stops the whole run."""


@dataclass(frozen=True)
class Project:
    id: str
    number: int
    title: str
    field_id: str
    options: dict[str, str]  # option name in lower case -> option id


def http_runner(token: str) -> Runner:
    """Return a runner that posts each query to the GraphQL API with `token`."""

    def run(query: str, variables: dict[str, Any]) -> dict[str, Any]:
        body = json.dumps({"query": query, "variables": variables}).encode()
        request = urllib.request.Request(
            GRAPHQL_URL,
            data=body,
            headers={
                "Authorization": f"Bearer {token}",
                "Content-Type": "application/json",
                "User-Agent": "fps-project-sync",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload: dict[str, Any] = json.load(response)
        except urllib.error.HTTPError as err:
            raise SyncError(f"GraphQL request failed with HTTP {err.code}") from err
        return payload

    return run


def call(run: Runner, query: str, variables: dict[str, Any]) -> dict[str, Any]:
    payload = run(query, variables)
    errors = payload.get("errors")
    if errors:
        messages = "; ".join(str(e.get("message", e)) for e in errors)
        raise SyncError(f"GraphQL error: {messages}")
    data: dict[str, Any] = payload.get("data") or {}
    return data


def to_project(node: dict[str, Any]) -> Project:
    field = node.get("field") or {}
    if not field.get("id"):
        raise SyncError(
            f"project {node['number']} has no single select field named {STATUS_FIELD}"
        )
    options = {o["name"].strip().lower(): o["id"] for o in field.get("options", [])}
    return Project(node["id"], node["number"], node["title"], field["id"], options)


def find_project(run: Runner, repo: str, login: str, number: int | None) -> Project:
    """Return the project with `number`, or else the one project of `login` linked to `repo`."""
    owner, name = repo.split("/", 1)
    if number is not None:
        data = call(run, Q_NUMBERED, {"login": login, "number": number})
        node = (data.get("user") or {}).get("projectV2")
        if not node:
            raise SyncError(f"user {login} has no project number {number}")
        return to_project(node)
    data = call(run, Q_LINKED, {"owner": owner, "name": name})
    nodes = ((data.get("repository") or {}).get("projectsV2") or {}).get("nodes") or []
    mine = [n for n in nodes if n and (n.get("owner") or {}).get("login") == login]
    if not mine:
        raise SyncError(f"no project of {login} is linked to {repo}")
    if len(mine) > 1:
        numbers = ", ".join(str(n["number"]) for n in mine)
        raise SyncError(
            f"{len(mine)} projects of {login} are linked to {repo} ({numbers});"
            " set the repository variable FPS_PROJECT_NUMBER"
        )
    return to_project(mine[0])


def iter_issues(run: Runner, repo: str, number: int | None) -> Iterator[dict[str, Any]]:
    owner, name = repo.split("/", 1)
    if number is not None:
        data = call(run, Q_ISSUE, {"owner": owner, "name": name, "number": number})
        issue = (data.get("repository") or {}).get("issue")
        if issue:
            yield issue
        return
    after: str | None = None
    while True:
        data = call(run, Q_ISSUES, {"owner": owner, "name": name, "after": after})
        page = data["repository"]["issues"]
        yield from page["nodes"]
        if not page["pageInfo"]["hasNextPage"]:
            return
        after = page["pageInfo"]["endCursor"]


def status_of(issue: dict[str, Any]) -> list[str]:
    names = [n["name"] for n in issue["labels"]["nodes"]]
    return [n for n in names if n in STATUS_OPTIONS]


def sync_issue(
    run: Runner, project: Project, issue: dict[str, Any], dry_run: bool, out: Any
) -> str:
    """Sync one issue and return what happened: set, unchanged, skipped, or failed."""
    number = issue["number"]
    labels = status_of(issue)
    if not labels:
        return "skipped"
    if len(labels) > 1:
        print(f"#{number}: skipped, more than one status label {labels}", file=out)
        return "failed"
    wanted = STATUS_OPTIONS[labels[0]]
    option = project.options.get(wanted.lower())
    if option is None:
        print(f"#{number}: no {STATUS_FIELD} option named {wanted!r}", file=out)
        return "failed"
    items = [
        i
        for i in issue["projectItems"]["nodes"]
        if i and (i.get("project") or {}).get("id") == project.id
    ]
    item = items[0] if items else None
    current = ((item or {}).get("fieldValueByName") or {}).get("optionId")
    if current == option:
        return "unchanged"
    prefix = "would set" if dry_run else "set"
    added = "" if item else ", added to the project"
    print(f"#{number}: {prefix} {STATUS_FIELD} to {wanted}{added}", file=out)
    if dry_run:
        return "set"
    if item is None:
        data = call(run, M_ADD, {"project": project.id, "content": issue["id"]})
        item_id = data["addProjectV2ItemById"]["item"]["id"]
    else:
        item_id = item["id"]
    call(
        run,
        M_SET,
        {
            "project": project.id,
            "item": item_id,
            "field": project.field_id,
            "option": option,
        },
    )
    return "set"


def event_issue_number(path: Path) -> int | None:
    """Return the issue number of an `issues` event payload, if it has one."""
    try:
        event = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    issue = event.get("issue") or {}
    if "pull_request" in issue:
        return None
    number = issue.get("number")
    return number if isinstance(number, int) else None


def sync(
    run: Runner,
    repo: str,
    login: str,
    project_number: int | None,
    issue_number: int | None,
    dry_run: bool,
    out: Any,
) -> int:
    project = find_project(run, repo, login, project_number)
    print(f"project {project.number} {project.title!r}", file=out)
    counts = {"set": 0, "unchanged": 0, "skipped": 0, "failed": 0}
    for issue in iter_issues(run, repo, issue_number):
        counts[sync_issue(run, project, issue, dry_run, out)] += 1
    summary = ", ".join(f"{v} {k}" for k, v in counts.items())
    print(f"done: {summary}", file=out)
    return 1 if counts["failed"] else 0


def parse_number(text: str | None, what: str) -> int | None:
    if text is None or not text.strip():
        return None
    try:
        return int(text)
    except ValueError as err:
        raise SyncError(f"{what} must be a number, got {text!r}") from err


def main(
    argv: list[str] | None = None,
    env: dict[str, str] | None = None,
    runner: Runner | None = None,
) -> int:
    env = dict(os.environ) if env is None else env
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--repo", default=env.get("FPS_REPO", "alrawi/FixingPointSystem")
    )
    parser.add_argument("--owner", default=env.get("FPS_PROJECT_OWNER", "alrawi"))
    parser.add_argument("--project", default=env.get("FPS_PROJECT_NUMBER"))
    parser.add_argument("--issue", help="sync only this issue number")
    parser.add_argument(
        "--event", type=Path, help="sync the issue of this event payload"
    )
    parser.add_argument(
        "--dry-run", action="store_true", help="print and change nothing"
    )
    args = parser.parse_args(argv)
    out = sys.stdout

    token = env.get("FPS_PROJECT_TOKEN", "").strip()
    if not token:
        print(
            "FPS_PROJECT_TOKEN is not set, so the project sync does nothing", file=out
        )
        return 0
    try:
        project_number = parse_number(args.project, "FPS_PROJECT_NUMBER")
        issue_number = parse_number(args.issue, "--issue")
        if issue_number is None and args.event is not None:
            issue_number = event_issue_number(args.event)
            if issue_number is None:
                print(
                    "the event names no issue, so the project sync does nothing",
                    file=out,
                )
                return 0
        return sync(
            runner or http_runner(token),
            args.repo,
            args.owner,
            project_number,
            issue_number,
            args.dry_run,
            out,
        )
    except SyncError as err:
        print(f"error: {err}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
