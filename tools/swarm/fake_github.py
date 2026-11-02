# SPDX-License-Identifier: GPL-3.0-only
"""In memory fake of the GitHub REST endpoints that fps_swarm.py uses.

The fake implements the `Runner` seam of fps_swarm.py. It answers the same
arguments that `gh api -i` receives and returns text in the format that
`gh api -i` prints, with a Date header taken from a controllable server clock.
Tests seed it from recorded responses under testdata/.
"""

from __future__ import annotations

import datetime as dt
import email.utils
import json
import threading
import urllib.parse
from collections.abc import Sequence
from pathlib import Path
from typing import Any

TESTDATA = Path(__file__).parent / "testdata"
REASONS = {
    200: "OK",
    201: "Created",
    204: "No Content",
    401: "Unauthorized",
    404: "Not Found",
}


class FakeGitHub:
    """A thread safe fake repository with issues, labels, comments, and a clock."""

    def __init__(
        self,
        repo: str = "alrawi/FixingPointSystem",
        now: dt.datetime | None = None,
        login: str | None = "alrawi",
    ) -> None:
        self.repo = repo
        self.now = now or dt.datetime(2026, 11, 2, 9, 0, 0, tzinfo=dt.timezone.utc)
        self.login = login
        self.issues: dict[int, dict[str, Any]] = {}
        self.comments: dict[int, list[dict[str, Any]]] = {}
        self.next_comment_id = 6583800000
        self.requests: list[tuple[str, str, Any]] = []
        self.lock = threading.Lock()

    # Seeding -------------------------------------------------------------

    def add_issue(
        self,
        number: int,
        title: str,
        labels: Sequence[str],
        state: str = "open",
        state_reason: str | None = None,
        body: str = "",
        pull_request: bool = False,
    ) -> dict[str, Any]:
        """Add an issue or pull request in the shape that the REST API returns."""
        issue: dict[str, Any] = {
            "number": number,
            "title": title,
            "state": state,
            "state_reason": state_reason,
            "body": body,
            "labels": [{"name": n} for n in labels],
            "assignees": [],
        }
        if pull_request:
            issue["pull_request"] = {
                "url": f"https://api.github.com/repos/{self.repo}/pulls/{number}"
            }
        self.issues[number] = issue
        self.comments.setdefault(number, [])
        return issue

    def load_recorded_issues(self, name: str = "recorded_issues.json") -> None:
        """Seed issues from a recorded listing under testdata/."""
        for issue in json.loads((TESTDATA / name).read_text(encoding="utf-8")):
            issue.setdefault("assignees", [])
            self.issues[int(issue["number"])] = issue
            self.comments.setdefault(int(issue["number"]), [])

    def add_comment(self, number: int, body: str, comment_id: int | None = None) -> int:
        """Add a comment directly, as if another client posted it."""
        with self.lock:
            return self._add_comment(number, body, comment_id)

    def close(self, number: int, reason: str = "completed") -> None:
        """Close an issue with a state reason."""
        self.issues[number]["state"] = "closed"
        self.issues[number]["state_reason"] = reason

    def advance(self, minutes: float) -> None:
        """Move the server clock forward."""
        self.now += dt.timedelta(minutes=minutes)

    def labels(self, number: int) -> list[str]:
        """Return the label names of an issue."""
        return [lab["name"] for lab in self.issues[number]["labels"]]

    def mutations(self) -> list[tuple[str, str, Any]]:
        """Return every request that is not a GET."""
        return [r for r in self.requests if r[0] != "GET"]

    # Runner --------------------------------------------------------------

    def __call__(self, args: Sequence[str], stdin: str | None) -> str:
        """Answer one `gh api` invocation."""
        argv = list(args)
        assert argv[0] == "-i", "fps_swarm must request headers for server time"
        method = argv[argv.index("--method") + 1]
        target = argv[argv.index("--method") + 2]
        payload = json.loads(stdin) if stdin else None
        parsed = urllib.parse.urlsplit(target)
        query = dict(urllib.parse.parse_qsl(parsed.query))
        with self.lock:
            self.requests.append((method, parsed.path, payload))
            status, body = self._route(method, parsed.path, query, payload)
        return self._render(status, body)

    def _render(self, status: int, body: Any) -> str:
        date = email.utils.format_datetime(
            self.now.astimezone(dt.timezone.utc), usegmt=True
        )
        head = [
            f"HTTP/1.1 {status} {REASONS.get(status, 'Error')}",
            "Content-Type: application/json; charset=utf-8",
            f"Date: {date}",
        ]
        text = "" if body is None else json.dumps(body)
        return "\n".join(head[:1]) + "\n" + "\r\n".join(head[1:]) + "\r\n\r\n" + text

    def _add_comment(
        self, number: int, body: str, comment_id: int | None = None
    ) -> int:
        cid = comment_id if comment_id is not None else self.next_comment_id
        self.next_comment_id = max(self.next_comment_id, cid) + 1
        self.comments.setdefault(number, []).append({"id": cid, "body": body})
        self.comments[number].sort(key=lambda c: c["id"])
        return cid

    def _route(
        self, method: str, path: str, query: dict[str, str], payload: Any
    ) -> tuple[int, Any]:
        base = f"repos/{self.repo}"
        if path == "user":
            return (
                (200, {"login": self.login})
                if self.login
                else (401, {"message": "Bad credentials"})
            )
        if path == base:
            return 200, {"full_name": self.repo}
        if not path.startswith(base + "/"):
            return 404, {"message": "Not Found"}
        parts = path[len(base) + 1 :].split("/")
        if parts == ["issues"] and method == "GET":
            return 200, self._page(self._list_issues(query), query)
        if (
            len(parts) == 3
            and parts[:2] == ["issues", "comments"]
            and method == "PATCH"
        ):
            for comments in self.comments.values():
                for c in comments:
                    if str(c["id"]) == parts[2]:
                        c["body"] = payload["body"]
                        return 200, c
            return 404, {"message": "Not Found"}
        if len(parts) < 2 or parts[0] != "issues" or not parts[1].isdigit():
            return 404, {"message": "Not Found"}
        number = int(parts[1])
        issue = self.issues.get(number)
        if issue is None:
            return 404, {"message": "Not Found"}
        rest = parts[2:]
        if rest == [] and method == "GET":
            return 200, issue
        if rest == ["comments"] and method == "GET":
            return 200, self._page(self.comments[number], query)
        if rest == ["comments"] and method == "POST":
            cid = self._add_comment(number, payload["body"])
            return 201, {"id": cid, "body": payload["body"]}
        if rest == ["labels"] and method == "POST":
            for name in payload["labels"]:
                if name not in self.labels(number):
                    issue["labels"].append({"name": name})
            return 200, issue["labels"]
        if len(rest) == 2 and rest[0] == "labels" and method == "DELETE":
            name = urllib.parse.unquote(rest[1])
            if name not in self.labels(number):
                return 404, {"message": "Label does not exist"}
            issue["labels"] = [lab for lab in issue["labels"] if lab["name"] != name]
            return 200, issue["labels"]
        if rest == ["assignees"] and method in ("POST", "DELETE"):
            names = set(payload["assignees"])
            current = [a["login"] for a in issue["assignees"]]
            current = (
                current + sorted(names - set(current))
                if method == "POST"
                else [a for a in current if a not in names]
            )
            issue["assignees"] = [{"login": a} for a in current]
            return 201 if method == "POST" else 200, issue
        return 404, {"message": "Not Found"}

    def _list_issues(self, query: dict[str, str]) -> list[dict[str, Any]]:
        state = query.get("state", "open")
        wanted = [w for w in query.get("labels", "").split(",") if w]
        found = []
        for number in sorted(self.issues, reverse=True):
            issue = self.issues[number]
            if state != "all" and issue["state"] != state:
                continue
            names = {lab["name"] for lab in issue["labels"]}
            if all(w in names for w in wanted):
                found.append(issue)
        return found

    @staticmethod
    def _page(items: list[Any], query: dict[str, str]) -> list[Any]:
        per_page = int(query.get("per_page", 30))
        page = int(query.get("page", 1))
        return items[(page - 1) * per_page : page * per_page]
