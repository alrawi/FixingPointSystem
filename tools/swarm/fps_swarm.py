# SPDX-License-Identifier: GPL-3.0-only
"""Run the Swarm Protocol over the GitHub REST API.

The command implements the claim, checkpoint, release, block, takeover, finish,
and sweep steps of docs/wiki/Swarm-Protocol.md. Every request goes through one
injectable runner that shells out to `gh api`, so tests replace it with a fake.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import email.utils
import functools
import json
import os
import re
import subprocess
import sys
import time
import urllib.parse
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from typing import Any

DEFAULT_REPO = "alrawi/FixingPointSystem"
LEASE_MINUTES = 90
SETTLE_SECONDS = 20
MIN_LEASE_LEFT_MINUTES = 10
PER_PAGE = 100
TIME_FORMAT = "%Y-%m-%dT%H:%M:%SZ"

CARD_STATES = ("active", "released", "blocked", "superseded", "done")
STATUS_LABELS = (
    "status:backlog",
    "status:ready",
    "status:in-progress",
    "status:blocked",
    "status:stale",
    "status:review",
    "status:done",
)

EXIT_OK = 0
EXIT_REFUSED = 1
EXIT_ERROR = 2

# ---------------------------------------------------------------------------
# Time helpers
# ---------------------------------------------------------------------------


def format_time(t: dt.datetime) -> str:
    """Format an aware datetime as the UTC string used in cards."""
    return t.astimezone(dt.timezone.utc).strftime(TIME_FORMAT)


def parse_time(text: str) -> dt.datetime:
    """Parse a card time such as 2026-11-02T09:30:00Z into an aware datetime."""
    return dt.datetime.strptime(text, TIME_FORMAT).replace(tzinfo=dt.timezone.utc)


def parse_http_date(text: str) -> dt.datetime:
    """Parse an HTTP Date header into an aware UTC datetime."""
    parsed = email.utils.parsedate_to_datetime(text.strip())
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=dt.timezone.utc)
    return parsed.astimezone(dt.timezone.utc)


# ---------------------------------------------------------------------------
# Status card
# ---------------------------------------------------------------------------

MARKER_RE = re.compile(
    r"^<!-- fps:card agent=(?P<agent>\S+) state=(?P<state>\S+) lease=(?P<lease>\S+) -->\s*$"
)
TITLE_RE = re.compile(
    r"^\*\*Status card\*\* for (?P<key>\S+), held by `(?P<agent>[^`]+)`\s*$"
)
ROW_RE = re.compile(r"^\|\s*(?P<field>[^|]+?)\s*\|\s*(?P<value>.*?)\s*\|\s*$")

FIELDS = (
    ("State", "state"),
    ("Lease expires (UTC)", "lease"),
    ("Progress", "progress"),
    ("Branch", "branch"),
    ("Last checkpoint", "last_checkpoint"),
    ("Pull request", "pull_request"),
    ("Handoff", "handoff"),
    ("Next action", "next_action"),
    ("Takeover of", "takeover_of"),
)
CODE_FIELDS = {"branch", "handoff"}


class CardError(ValueError):
    """A comment looks like a status card but does not follow the format."""


@dataclass(frozen=True)
class Card:
    """One status card, as rendered in an issue comment.

    `pull_request` holds the number without `#`, or "none".
    """

    key: str
    agent: str
    state: str
    lease: dt.datetime
    progress: str = "0% (0 of 0 plan steps)"
    branch: str = ""
    last_checkpoint: str = "none"
    pull_request: str = "none"
    handoff: str = ""
    next_action: str = "none"
    takeover_of: str = "none"

    def expired(self, now: dt.datetime) -> bool:
        """Return whether the lease has run out at server time `now`."""
        return now >= self.lease

    def render(self) -> str:
        """Render the card as the comment body that the Swarm Protocol defines."""
        if self.state not in CARD_STATES:
            raise CardError(f"card for {self.key}: unknown state {self.state}")
        lines = [
            f"<!-- fps:card agent={self.agent} state={self.state} lease={format_time(self.lease)} -->",
            f"**Status card** for {self.key}, held by `{self.agent}`",
            "",
            "| Field | Value |",
            "|---|---|",
        ]
        for label, attr in FIELDS:
            lines.append(f"| {label} | {_cell(self._display(attr))} |")
        return "\n".join(lines) + "\n"

    def _display(self, attr: str) -> str:
        if attr == "lease":
            return format_time(self.lease)
        value = str(getattr(self, attr))
        if attr == "pull_request" and value not in ("", "none"):
            return f"#{value}"
        if attr in CODE_FIELDS and value:
            return f"`{value}`"
        return value


def _cell(value: str) -> str:
    return " ".join(value.split()).replace("|", "\\|")


def _uncell(value: str) -> str:
    return value.replace("\\|", "|")


def is_card(body: str) -> bool:
    """Return whether a comment body is a status card."""
    return body.startswith("<!-- fps:card")


def parse_card(body: str) -> Card:
    """Parse a status card comment body. The marker line is authoritative."""
    lines = body.replace("\r\n", "\n").split("\n")
    marker = MARKER_RE.match(lines[0]) if lines else None
    if marker is None:
        raise CardError("status card: first line is not a valid fps:card marker")
    state = marker["state"]
    if state not in CARD_STATES:
        raise CardError(f"status card of {marker['agent']}: unknown state {state}")
    try:
        lease = parse_time(marker["lease"])
    except ValueError as err:
        raise CardError(
            f"status card of {marker['agent']}: bad lease {marker['lease']}"
        ) from err
    key = ""
    values: dict[str, str] = {}
    labels = {label: attr for label, attr in FIELDS}
    for line in lines[1:]:
        title = TITLE_RE.match(line)
        if title:
            key = title["key"]
            continue
        row = ROW_RE.match(line)
        if row and row["field"] in labels:
            values[labels[row["field"]]] = _uncell(row["value"])
    for attr in CODE_FIELDS:
        if attr in values:
            values[attr] = values[attr].strip("`")
    if values.get("pull_request", "").startswith("#"):
        values["pull_request"] = values["pull_request"][1:]
    values.pop("state", None)
    values.pop("lease", None)
    return Card(key=key, agent=marker["agent"], state=state, lease=lease, **values)


@dataclass(frozen=True)
class CardComment:
    """A status card together with the id of the comment that carries it."""

    comment_id: int
    card: Card


def cards_from_comments(comments: Iterable[dict[str, Any]]) -> list[CardComment]:
    """Extract every parseable card from issue comments, ordered by comment id.

    Comments that start with the marker but do not parse are skipped, because
    the holder rule must not fail on one malformed comment.
    """
    found = []
    for c in comments:
        body = c.get("body") or ""
        if not is_card(body):
            continue
        try:
            found.append(CardComment(int(c["id"]), parse_card(body)))
        except CardError:
            continue
    return sorted(found, key=lambda cc: cc.comment_id)


def holder(cards: Sequence[CardComment], now: dt.datetime) -> CardComment | None:
    """Apply the holder rule.

    Among cards whose state is active and whose lease has not expired, the card
    with the lowest comment id holds the lease.
    """
    live = [
        cc for cc in cards if cc.card.state == "active" and not cc.card.expired(now)
    ]
    return min(live, key=lambda cc: cc.comment_id) if live else None


def own_card(cards: Sequence[CardComment], agent: str) -> CardComment | None:
    """Return the newest card that `agent` posted, or None."""
    mine = [cc for cc in cards if cc.card.agent == agent]
    return mine[-1] if mine else None


# ---------------------------------------------------------------------------
# API seam
# ---------------------------------------------------------------------------


@dataclass
class Response:
    """One REST response as `gh api -i` reports it."""

    status: int
    headers: dict[str, str]
    body: Any


Runner = Callable[[Sequence[str], str | None], str]
"""Run `gh api` with the given arguments and stdin, and return its stdout."""

HEADER_END_RE = re.compile(r"\r?\n\r?\n")
STATUS_RE = re.compile(r"^HTTP/\S+\s+(\d{3})")


def gh_runner(args: Sequence[str], stdin: str | None) -> str:
    """Run `gh api` in a subprocess. This is the only place that reaches the network."""
    res = subprocess.run(
        ["gh", "api", *args], input=stdin, capture_output=True, text=True, check=False
    )
    if not res.stdout.startswith("HTTP/"):
        raise ApiError(
            0, f"gh api {' '.join(args)}: {res.stderr.strip() or 'no response'}"
        )
    return res.stdout


def parse_response(raw: str) -> Response:
    """Parse the output of `gh api -i` into status, headers, and JSON body."""
    parts = HEADER_END_RE.split(raw, maxsplit=1)
    head = parts[0].splitlines()
    text = parts[1] if len(parts) > 1 else ""
    status_match = STATUS_RE.match(head[0]) if head else None
    if status_match is None:
        raise ApiError(0, "gh api: response has no HTTP status line")
    headers = {}
    for line in head[1:]:
        name, sep, value = line.partition(":")
        if sep:
            headers[name.strip().lower()] = value.strip()
    body: Any = None
    if text.strip():
        try:
            body = json.loads(text)
        except json.JSONDecodeError:
            body = text
    return Response(int(status_match[1]), headers, body)


class ApiError(RuntimeError):
    """A REST call returned an error status."""

    def __init__(self, status: int, message: str) -> None:
        super().__init__(message)
        self.status = status


class Api:
    """Repository scoped REST client on top of one injectable runner."""

    def __init__(self, repo: str, runner: Runner = gh_runner) -> None:
        self.repo = repo
        self.runner = runner
        self.last_date: dt.datetime | None = None

    def call(
        self,
        method: str,
        path: str,
        payload: Any = None,
        query: dict[str, Any] | None = None,
    ) -> Response:
        """Send one request. Paths without a leading `repos/` are relative to the repository."""
        if not path.startswith("repos/") and path != "user":
            path = f"repos/{self.repo}/{path}" if path else f"repos/{self.repo}"
        if query:
            path = f"{path}?{urllib.parse.urlencode(query)}"
        args = ["-i", "--method", method, path]
        stdin = None
        if payload is not None:
            args += ["--input", "-"]
            stdin = json.dumps(payload)
        resp = parse_response(self.runner(args, stdin))
        if "date" in resp.headers:
            self.last_date = parse_http_date(resp.headers["date"])
        if resp.status >= 400:
            message = (
                resp.body.get("message", "") if isinstance(resp.body, dict) else ""
            )
            raise ApiError(
                resp.status, f"{method} {path}: HTTP {resp.status} {message}".strip()
            )
        return resp

    def get(self, path: str, **query: Any) -> Any:
        """GET one resource and return its JSON body."""
        return self.call("GET", path, query=query or None).body

    def get_all(self, path: str, **query: Any) -> list[Any]:
        """GET every page of a list endpoint."""
        items: list[Any] = []
        page = 1
        while True:
            body = self.get(path, **query, per_page=PER_PAGE, page=page)
            if not isinstance(body, list):
                raise ApiError(0, f"GET {path}: expected a list")
            items += body
            if len(body) < PER_PAGE:
                return items
            page += 1

    def now(self) -> dt.datetime:
        """Return GitHub server time from the Date header of a fresh response."""
        self.call("GET", "")
        if self.last_date is None:
            raise ApiError(0, "server time: response carried no Date header")
        return self.last_date

    # Issue helpers -------------------------------------------------------

    def issue(self, number: int) -> dict[str, Any]:
        """Return one issue."""
        body: dict[str, Any] = self.get(f"issues/{number}")
        return body

    def comments(self, number: int) -> list[dict[str, Any]]:
        """Return every comment of an issue."""
        return self.get_all(f"issues/{number}/comments")

    def cards(self, number: int) -> list[CardComment]:
        """Return every status card on an issue, ordered by comment id."""
        return cards_from_comments(self.comments(number))

    def post_comment(self, number: int, body: str) -> int:
        """Post an issue comment and return its id."""
        return int(
            self.call("POST", f"issues/{number}/comments", {"body": body}).body["id"]
        )

    def edit_comment(self, comment_id: int, body: str) -> None:
        """Replace the body of an issue comment."""
        self.call("PATCH", f"issues/comments/{comment_id}", {"body": body})

    def add_labels(self, number: int, labels: Sequence[str]) -> None:
        """Add labels to an issue or pull request."""
        self.call("POST", f"issues/{number}/labels", {"labels": list(labels)})

    def remove_label(self, number: int, label: str) -> None:
        """Remove one label. A 404 means that the label is already gone."""
        try:
            self.call(
                "DELETE", f"issues/{number}/labels/{urllib.parse.quote(label, safe='')}"
            )
        except ApiError as err:
            if err.status != 404:
                raise

    def set_status(self, number: int, new: str, current: Iterable[str]) -> None:
        """Replace every status label in `current` with `new`."""
        for label in current:
            if label.startswith("status:") and label != new:
                self.remove_label(number, label)
        self.add_labels(number, [new])


# ---------------------------------------------------------------------------
# Work items
# ---------------------------------------------------------------------------

TITLE_ID_RE = re.compile(r"^\[(?P<id>[A-Z]+-[0-9]+)\]")
DEPENDS_RE = re.compile(r"^- \*\*Depends on\.\*\* (?P<deps>.*)$", re.MULTILINE)
BLOCK_MARKER_RE = re.compile(
    r"^<!-- fps:block agent=(?P<agent>\S+) issues=(?P<issues>\S*) -->"
)


def labels_of(issue: dict[str, Any]) -> list[str]:
    """Return the label names of an issue."""
    return [
        lab["name"] if isinstance(lab, dict) else str(lab)
        for lab in issue.get("labels", [])
    ]


@dataclass(frozen=True)
class WorkItem:
    """A claimable issue with the names that the protocol derives from it."""

    number: int
    title: str
    labels: tuple[str, ...]
    key: str
    branch: str
    handoff: str

    @staticmethod
    def from_issue(issue: dict[str, Any]) -> WorkItem:
        """Derive key, branch, and handoff path from an issue."""
        number = int(issue["number"])
        labels = tuple(labels_of(issue))
        title = str(issue.get("title", ""))
        match = TITLE_ID_RE.match(title)
        if "type:bug" in labels or match is None:
            key, branch = f"bug-{number}", f"bug/{number}"
        else:
            key, branch = match["id"], f"wp/{match['id']}"
        return WorkItem(number, title, labels, key, branch, f"docs/handoff/{key}.md")


def depends_on(body: str) -> list[str]:
    """Return the dependency ids listed in an issue body."""
    match = DEPENDS_RE.search(body or "")
    if match is None:
        return []
    deps = match["deps"].strip()
    if deps.lower() == "none":
        return []
    return [d.strip() for d in deps.split(",") if d.strip()]


def wp_index(api: Api) -> dict[str, dict[str, Any]]:
    """Map catalog ids to their work package issues, open and closed."""
    index = {}
    for issue in api.get_all("issues", state="all", labels="type:wp"):
        match = TITLE_ID_RE.match(issue.get("title", ""))
        if match and "pull_request" not in issue:
            index[match["id"]] = issue
    return index


def closed_completed(issue: dict[str, Any] | None) -> bool:
    """Return whether an issue closed with state_reason completed."""
    return (
        issue is not None
        and issue.get("state") == "closed"
        and issue.get("state_reason") == "completed"
    )


def is_claimable(issue: dict[str, Any]) -> bool:
    """Apply the claimable status rule of the Swarm Protocol."""
    labels = labels_of(issue)
    return (
        issue.get("state") == "open"
        and "pull_request" not in issue
        and ("status:ready" in labels or "status:stale" in labels)
        and "needs:split" not in labels
        and "type:followup" not in labels
        and ("type:wp" in labels or "type:bug" in labels)
    )


def _phase(labels: Sequence[str]) -> int:
    for label in labels:
        if label.startswith("phase:") and label[6:].isdigit():
            return int(label[6:])
    return 99


def selection_key(issue: dict[str, Any]) -> tuple[int, int, int, int]:
    """Order claimable issues by the selection rule, leaving out the review group."""
    labels = labels_of(issue)
    stale = "status:stale" in labels
    if "critical-path" in labels:
        group = 0 if stale else 1
    else:
        group = 2 if stale else 3
    kind = 0 if "type:bug" in labels else 1
    return (group, kind, _phase(labels), int(issue["number"]))


# ---------------------------------------------------------------------------
# Operations
# ---------------------------------------------------------------------------


class Refused(Exception):
    """The protocol does not allow the requested step."""


Sleep = Callable[[float], None]
Log = Callable[[str], None]


def _stderr(message: str) -> None:
    print(message, file=sys.stderr)


@dataclass
class Swarm:
    """The protocol operations of one agent on one repository."""

    api: Api
    agent: str
    sleep: Sleep = time.sleep
    log: Log = _stderr
    login: str | None = None

    # Reading -----------------------------------------------------------

    def list_claimable(self) -> list[dict[str, Any]]:
        """Return claimable issues in selection order."""
        seen: dict[int, dict[str, Any]] = {}
        for kind in ("type:wp", "type:bug"):
            for status in ("status:stale", "status:ready"):
                for issue in self.api.get_all(
                    "issues", state="open", labels=f"{kind},{status}"
                ):
                    if is_claimable(issue):
                        seen[int(issue["number"])] = issue
        return sorted(seen.values(), key=selection_key)

    def dependencies_met(self, issue: dict[str, Any]) -> tuple[bool, list[str]]:
        """Return whether every dependency closed as completed, and the open ones."""
        deps = depends_on(issue.get("body") or "")
        if not deps:
            return True, []
        index = wp_index(self.api)
        missing = [d for d in deps if not closed_completed(index.get(d))]
        return not missing, missing

    # Claim and takeover --------------------------------------------------

    def precheck(
        self, number: int, takeover: bool = False
    ) -> tuple[WorkItem, list[CardComment]]:
        """Run claim steps 1 and 2 and return the work item and its cards."""
        issue = self.api.issue(number)
        labels = labels_of(issue)
        if takeover:
            allowed = {"status:ready", "status:stale", "status:in-progress"}
            if issue.get("state") != "open" or not allowed & set(labels):
                raise Refused(
                    f"issue #{number}: not open with a ready, stale, or in-progress label"
                )
            if "needs:split" in labels or "type:followup" in labels:
                raise Refused(
                    f"issue #{number}: needs a split or triage before any claim"
                )
        elif not is_claimable(issue):
            raise Refused(
                f"issue #{number}: not claimable with labels {','.join(labels)}"
            )
        met, missing = self.dependencies_met(issue)
        if not met:
            raise Refused(
                f"issue #{number}: dependencies not closed as completed: {', '.join(missing)}"
            )
        cards = self.api.cards(number)
        now = self.api.now()
        current = holder(cards, now)
        if current is not None:
            raise Refused(
                f"issue #{number}: card {current.comment_id} of {current.card.agent} holds the lease"
            )
        if takeover and not cards:
            raise Refused(f"issue #{number}: no earlier card, use claim")
        return WorkItem.from_issue(issue), cards

    def post_card(
        self, item: WorkItem, cards: Sequence[CardComment], steps: int = 0
    ) -> CardComment:
        """Run claim step 3 and return the new card."""
        now = self.api.now()
        previous = cards[-1].card.agent if cards else "none"
        card = Card(
            key=item.key,
            agent=self.agent,
            state="active",
            lease=now + dt.timedelta(minutes=LEASE_MINUTES),
            progress=f"0% (0 of {steps} plan steps)",
            branch=item.branch,
            handoff=item.handoff,
            next_action="Push the claim commit and open the pull request",
            takeover_of=previous,
        )
        cid = self.api.post_comment(item.number, card.render())
        self.log(
            f"posted card {cid} on #{item.number} with lease {format_time(card.lease)}"
        )
        return CardComment(cid, card)

    def settle(self, item: WorkItem, mine: CardComment) -> bool:
        """Run claim steps 4 and 5 after the settle wait. Return whether we won."""
        cards = self.api.cards(item.number)
        now = self.api.now()
        current = holder(cards, now)
        if current is None or current.comment_id != mine.comment_id:
            lost = dataclasses.replace(mine.card, state="superseded")
            self.api.edit_comment(mine.comment_id, lost.render())
            who = (
                f"card {current.comment_id} of {current.card.agent}"
                if current
                else "no card"
            )
            self.log(f"lost #{item.number} to {who}, card {mine.comment_id} superseded")
            return False
        issue = self.api.issue(item.number)
        self.api.set_status(item.number, "status:in-progress", labels_of(issue))
        if self.login:
            try:
                self.api.call(
                    "POST",
                    f"issues/{item.number}/assignees",
                    {"assignees": [self.login]},
                )
            except ApiError as err:
                self.log(f"assignee not set on #{item.number}: {err}")
        return True

    def claim(
        self, number: int, takeover: bool = False, steps: int = 0
    ) -> CardComment | None:
        """Claim an item with the seven step procedure, leaving git to the caller.

        Return the winning card, or None when another agent won the settle race.
        """
        item, cards = self.precheck(number, takeover=takeover)
        mine = self.post_card(item, cards, steps=steps)
        self.sleep(SETTLE_SECONDS)
        return mine if self.settle(item, mine) else None

    # Holding -------------------------------------------------------------

    def require_holder(
        self, number: int, min_left: int = MIN_LEASE_LEFT_MINUTES
    ) -> CardComment:
        """Return our card when it holds the lease with `min_left` minutes left."""
        cards = self.api.cards(number)
        now = self.api.now()
        current = holder(cards, now)
        if current is None or current.card.agent != self.agent:
            who = f"{current.card.agent}" if current else "nobody"
            raise Refused(
                f"issue #{number}: {self.agent} does not hold the lease, holder is {who}"
            )
        left = current.card.lease - now
        if left < dt.timedelta(minutes=min_left):
            raise Refused(
                f"issue #{number}: lease has {int(left.total_seconds() // 60)} minutes left,"
                f" below {min_left}"
            )
        return current

    def heartbeat(self, number: int, **updates: str | None) -> CardComment:
        """Renew the lease and update card fields at a checkpoint."""
        mine = self.require_holder(number)
        now = self.api.now()
        fields_ = {k: v for k, v in updates.items() if v is not None}
        card = dataclasses.replace(
            mine.card, lease=now + dt.timedelta(minutes=LEASE_MINUTES), **fields_
        )
        self.api.edit_comment(mine.comment_id, card.render())
        self.log(
            f"renewed card {mine.comment_id} on #{number} until {format_time(card.lease)}"
        )
        return CardComment(mine.comment_id, card)

    def _finish_card(
        self, number: int, state: str, **updates: str | None
    ) -> CardComment:
        mine = self.require_holder(number, min_left=0)
        fields_: dict[str, Any] = {k: v for k, v in updates.items() if v is not None}
        fields_["state"] = state
        card = dataclasses.replace(mine.card, **fields_)
        self.api.edit_comment(mine.comment_id, card.render())
        return CardComment(mine.comment_id, card)

    def release(self, number: int, next_action: str | None = None) -> CardComment:
        """Release the lease and return the item to status:ready with has-handoff."""
        done = self._finish_card(number, "released", next_action=next_action)
        issue = self.api.issue(number)
        self.api.set_status(number, "status:ready", labels_of(issue))
        self.api.add_labels(number, ["has-handoff"])
        if self.login:
            self.api.call(
                "DELETE", f"issues/{number}/assignees", {"assignees": [self.login]}
            )
        self.log(f"released #{number}, card {done.comment_id}")
        return done

    def block(
        self,
        number: int,
        reason: str,
        blocked_by: Sequence[int] = (),
        needs_owner: bool = False,
        needs_split: bool = False,
    ) -> CardComment:
        """Block the item with a blocking comment that the promotion sweep can read."""
        self.require_holder(number, min_left=0)
        refs = ",".join(str(n) for n in blocked_by)
        lines = [
            f"<!-- fps:block agent={self.agent} issues={refs} -->",
            f"**Blocked** by `{self.agent}`",
            "",
        ]
        lines.append(" ".join(reason.split()))
        if blocked_by:
            lines += ["", "Blocking issues: " + ", ".join(f"#{n}" for n in blocked_by)]
        self.api.post_comment(number, "\n".join(lines) + "\n")
        done = self._finish_card(number, "blocked")
        issue = self.api.issue(number)
        self.api.set_status(number, "status:blocked", labels_of(issue))
        extra = (["needs:owner"] if needs_owner or needs_split else []) + (
            ["needs:split"] if needs_split else []
        )
        if extra:
            self.api.add_labels(number, extra)
        self.log(f"blocked #{number}, card {done.comment_id}")
        return done

    def review(self, number: int, pull_request: int | None = None) -> CardComment:
        """Finish the item. Remove `wip`, set status:review, and mark the card done."""
        mine = self.require_holder(number, min_left=0)
        pr = pull_request
        if pr is None and mine.card.pull_request.isdigit():
            pr = int(mine.card.pull_request)
        if pr is None:
            raise Refused(f"issue #{number}: no pull request on the card, pass --pr")
        self.api.remove_label(pr, "wip")
        issue = self.api.issue(number)
        self.api.set_status(number, "status:review", labels_of(issue))
        done = self._finish_card(number, "done", pull_request=str(pr))
        self.log(
            f"#{number} in review with pull request #{pr}, card {done.comment_id} done"
        )
        return done

    # Sweeps --------------------------------------------------------------

    def stale_sweep(self, dry_run: bool = False) -> list[int]:
        """Relabel in progress items whose holder card expired as status:stale."""
        changed = []
        now = self.api.now()
        for issue in self.api.get_all(
            "issues", state="open", labels="status:in-progress"
        ):
            if "pull_request" in issue:
                continue
            number = int(issue["number"])
            cards = self.api.cards(number)
            if holder(cards, now) is not None:
                continue
            changed.append(number)
            self.log(f"stale: #{number} has no unexpired active card")
            if not dry_run:
                self._relabel(number, "status:stale", labels_of(issue))
        return changed

    def promotion_sweep(self, dry_run: bool = False) -> list[tuple[int, str]]:
        """Apply the three promotion rules and return (issue, new status) pairs."""
        changes: list[tuple[int, str]] = []
        index = wp_index(self.api)
        for issue in self.api.get_all(
            "issues", state="open", labels="type:wp,status:backlog"
        ):
            labels = labels_of(issue)
            if (
                "pull_request" in issue
                or "needs:split" in labels
                or "type:followup" in labels
            ):
                continue
            deps = depends_on(issue.get("body") or "")
            if all(closed_completed(index.get(d)) for d in deps):
                changes.append((int(issue["number"]), "status:ready"))
                if not dry_run:
                    self._relabel(int(issue["number"]), "status:ready", labels)
        for issue in self.api.get_all("issues", state="open", labels="status:blocked"):
            if "pull_request" in issue:
                continue
            number = int(issue["number"])
            refs = self._blocking_issues(number)
            if refs is None or not refs:
                continue
            if all(closed_completed(self._issue_or_none(n)) for n in refs):
                changes.append((number, "status:ready"))
                if not dry_run:
                    self._relabel(number, "status:ready", labels_of(issue))
                    self._ignore_404(
                        functools.partial(self.api.add_labels, number, ["has-handoff"])
                    )
        for status in STATUS_LABELS:
            if status == "status:done":
                continue
            for issue in self.api.get_all("issues", state="closed", labels=status):
                if "pull_request" in issue:
                    continue
                number = int(issue["number"])
                changes.append((number, "status:done"))
                if not dry_run:
                    self._relabel(number, "status:done", labels_of(issue))
        for number, status in changes:
            self.log(f"promote: #{number} to {status}{' (dry run)' if dry_run else ''}")
        return changes

    def _blocking_issues(self, number: int) -> list[int] | None:
        newest = None
        for c in self.api.comments(number):
            match = BLOCK_MARKER_RE.match(c.get("body") or "")
            if match:
                newest = match
        if newest is None:
            return None
        return [int(n) for n in newest["issues"].split(",") if n.isdigit()]

    def _issue_or_none(self, number: int) -> dict[str, Any] | None:
        try:
            return self.api.issue(number)
        except ApiError as err:
            if err.status == 404:
                return None
            raise

    def _relabel(self, number: int, new: str, current: Sequence[str]) -> None:
        self._ignore_404(lambda: self.api.set_status(number, new, current))

    @staticmethod
    def _ignore_404(action: Callable[[], None]) -> None:
        try:
            action()
        except ApiError as err:
            if err.status != 404:
                raise


# ---------------------------------------------------------------------------
# Command line
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    """Build the argument parser of the command."""
    p = argparse.ArgumentParser(
        prog="fps_swarm.py", description=__doc__.splitlines()[0]
    )
    p.add_argument("--repo", default=os.environ.get("FPS_REPO", DEFAULT_REPO))
    p.add_argument(
        "--agent",
        default=os.environ.get("FPS_AGENT_ID"),
        help="defaults to $FPS_AGENT_ID",
    )
    sub = p.add_subparsers(dest="command", required=True)

    s = sub.add_parser("list", help="print claimable items in selection order")
    s.add_argument("--json", action="store_true")

    for name, text in (
        ("claim", "claim a ready or stale item"),
        ("takeover", "take over an expired, released, or blocked item"),
    ):
        s = sub.add_parser(name, help=text)
        s.add_argument("issue", type=int)
        s.add_argument(
            "--steps",
            type=int,
            default=0,
            help="number of plan steps for the progress field",
        )
        s.add_argument(
            "--no-assign", action="store_true", help="skip the assignee step"
        )

    s = sub.add_parser("heartbeat", help="checkpoint the card and renew the lease")
    s.add_argument("issue", type=int)
    s.add_argument(
        "--check",
        action="store_true",
        help="only confirm the lease, with 10 minutes left",
    )
    s.add_argument("--progress")
    s.add_argument("--commit", dest="last_checkpoint")
    s.add_argument("--pr", dest="pull_request")
    s.add_argument("--next", dest="next_action")

    s = sub.add_parser("release", help="release the lease and return the item to ready")
    s.add_argument("issue", type=int)
    s.add_argument("--next", dest="next_action")
    s.add_argument(
        "--no-assign", action="store_true", help="skip removing the assignee"
    )

    s = sub.add_parser("block", help="block the item on a named issue or owner action")
    s.add_argument("issue", type=int)
    s.add_argument("--reason", required=True)
    s.add_argument("--blocked-by", type=int, action="append", default=[])
    s.add_argument("--needs-owner", action="store_true")
    s.add_argument("--needs-split", action="store_true")

    s = sub.add_parser("review", help="finish the item and move it to status:review")
    s.add_argument("issue", type=int)
    s.add_argument("--pr", type=int)

    for name, text in (
        ("promote", "run the promotion sweep"),
        ("stale", "run the stale sweep"),
        ("sweep", "run both sweeps"),
    ):
        s = sub.add_parser(name, help=text)
        s.add_argument("--dry-run", action="store_true")

    s = sub.add_parser("now", help="print GitHub server time")
    return p


NEEDS_AGENT = {"claim", "takeover", "heartbeat", "release", "block", "review"}


def main(
    argv: list[str] | None = None, runner: Runner = gh_runner, sleep: Sleep = time.sleep
) -> int:
    """Run one subcommand and return the exit code."""
    args = build_parser().parse_args(argv)
    if args.command in NEEDS_AGENT and not args.agent:
        _stderr("fps_swarm: set --agent or FPS_AGENT_ID")
        return EXIT_ERROR
    api = Api(args.repo, runner)
    swarm = Swarm(api, args.agent or "", sleep=sleep)
    try:
        return _dispatch(args, api, swarm)
    except Refused as err:
        _stderr(f"fps_swarm: {err}")
        return EXIT_REFUSED
    except ApiError as err:
        _stderr(f"fps_swarm: {err}")
        return EXIT_ERROR


def _resolve_login(api: Api) -> str | None:
    try:
        login = api.get("user").get("login")
    except (ApiError, AttributeError):
        return None
    return str(login) if login else None


def _dispatch(args: argparse.Namespace, api: Api, swarm: Swarm) -> int:
    cmd = args.command
    if cmd == "now":
        print(format_time(api.now()))
    elif cmd == "list":
        items = swarm.list_claimable()
        if args.json:
            print(
                json.dumps(
                    [dataclasses.asdict(WorkItem.from_issue(i)) for i in items],
                    indent=2,
                )
            )
        for i in [] if args.json else items:
            w = WorkItem.from_issue(i)
            print(f"{w.number}\t{w.key}\t{w.title}\t{','.join(w.labels)}")
    elif cmd in ("claim", "takeover"):
        if not args.no_assign:
            swarm.login = _resolve_login(api)
        won = swarm.claim(args.issue, takeover=cmd == "takeover", steps=args.steps)
        if won is None:
            return EXIT_REFUSED
        c = won.card
        print(
            f"card={won.comment_id} key={c.key} branch={c.branch} handoff={c.handoff} lease={format_time(c.lease)}"
        )
    elif cmd == "heartbeat":
        if args.check:
            mine = swarm.require_holder(args.issue)
        else:
            mine = swarm.heartbeat(
                args.issue,
                progress=args.progress,
                last_checkpoint=args.last_checkpoint,
                pull_request=args.pull_request.lstrip("#")
                if args.pull_request
                else None,
                next_action=args.next_action,
            )
        print(f"card={mine.comment_id} lease={format_time(mine.card.lease)}")
    elif cmd == "release":
        if not args.no_assign:
            swarm.login = _resolve_login(api)
        swarm.release(args.issue, next_action=args.next_action)
    elif cmd == "block":
        swarm.block(
            args.issue, args.reason, args.blocked_by, args.needs_owner, args.needs_split
        )
    elif cmd == "review":
        swarm.review(args.issue, args.pr)
    elif cmd in ("promote", "stale", "sweep"):
        if cmd in ("promote", "sweep"):
            for number, status in swarm.promotion_sweep(dry_run=args.dry_run):
                print(f"{number}\t{status}")
        if cmd in ("stale", "sweep"):
            for number in swarm.stale_sweep(dry_run=args.dry_run):
                print(f"{number}\tstatus:stale")
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
