# SPDX-License-Identifier: GPL-3.0-only
"""Keep the self hosted runner of the owner host off untrusted triggers.

The repository is public, and the owner's server fat-nuc runs a self hosted
runner with the label fps-bench. A workflow that names such a runner and that a
push, a pull request, a comment, or another workflow can start would run code
on that server before anyone reviews it. Rule 1 and rule 2 of the section
"Owner host" of docs/wiki/Operations.md, and decision D13, therefore allow a
self hosted job only when all of these hold:

1. The workflow starts only on `workflow_dispatch` and `schedule`.
2. The job, or the workflow, has `permissions` with `contents: read` and no
   write permission.
3. Every `actions/checkout` step of the job sets `persist-credentials: false`,
   and the job has at least one such step.
4. The job sets `timeout-minutes` to a positive whole number.

A job is self hosted unless every `runs-on` label is a GitHub hosted label
(`ubuntu-*`, `windows-*`, `macos-*`). A label that is a group, an expression
that this check cannot resolve, or a name such as `self-hosted` or `fps-bench`
makes the job self hosted, so a typo fails closed.

A job level `if:` is NOT accepted in place of rule 1. A guard such as
`github.event_name == 'schedule'` lives in the same file as the triggers, and
one wrong clause sends code of a fork or of an unreviewed branch to the owner
host, while the trigger list is a plain set that this check reads exactly. A
workflow with a self hosted job and a `push` or `pull_request` trigger fails,
whatever its `if:` says, and the job moves to a workflow of its own. The
`if:` stays useful as a second layer, because `workflow_dispatch` also starts
a run of the file on any branch, and only `github.ref == 'refs/heads/main'`
keeps that off the owner host. The check cannot prove an expression, so it only
warns on stderr when a self hosted job has no `if:` that names
`github.ref == 'refs/heads/main'`. The warning does not change the exit code.

Usage: check_self_hosted.py [--workflows DIR]

Exit codes are 0 when every workflow passes, 1 when a workflow breaks a rule,
and 2 when a workflow file cannot be read or parsed. The command needs PyYAML
and no network.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github" / "workflows"

ALLOWED_TRIGGERS = frozenset({"workflow_dispatch", "schedule"})
HOSTED_LABEL = re.compile(r"^(ubuntu|windows|macos)-[A-Za-z0-9._-]+$")
MATRIX_REF = re.compile(r"^\$\{\{\s*matrix\.([A-Za-z_][\w-]*)\s*\}\}$")
MAIN_GUARD = "github.ref == 'refs/heads/main'"


class WorkflowError(Exception):
    """A workflow file that this check cannot read."""


@dataclass(frozen=True)
class Finding:
    """One broken rule of one job, or of the workflow when job is empty."""

    file: str
    job: str
    rule: str
    message: str

    def __str__(self) -> str:
        where = f"{self.file}:{self.job}" if self.job else self.file
        return f"{where}: {self.rule}: {self.message}"


def load_workflow(path: Path) -> dict[str, Any]:
    try:
        data = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, yaml.YAMLError) as exc:
        raise WorkflowError(f"{path.name}: cannot read workflow: {exc}") from exc
    if not isinstance(data, dict):
        raise WorkflowError(f"{path.name}: a workflow must be a mapping")
    return data


def triggers_of(workflow: dict[Any, Any]) -> set[str]:
    """Names of the events that start the workflow.

    YAML 1.1 reads the key `on` as the boolean True, so both spellings count.
    The key may hold a name, a list of names, or a mapping of names.
    """
    if "on" in workflow:
        raw = workflow["on"]
    elif True in workflow:
        raw = workflow[True]
    else:
        return set()
    if isinstance(raw, str):
        return {raw}
    if isinstance(raw, list):
        return {str(item) for item in raw}
    if isinstance(raw, dict):
        return {str(key) for key in raw}
    return {str(raw)}


def _matrix_values(job: dict[str, Any], key: str) -> list[Any] | None:
    """Static values of matrix.<key>, or None when the matrix is not literal."""
    strategy = job.get("strategy")
    matrix = strategy.get("matrix") if isinstance(strategy, dict) else None
    if not isinstance(matrix, dict):
        return None
    values: list[Any] = []
    if key in matrix:
        axis = matrix[key]
        if not isinstance(axis, list):
            return None
        values.extend(axis)
    include = matrix.get("include")
    if isinstance(include, list):
        for entry in include:
            if isinstance(entry, dict) and key in entry:
                values.append(entry[key])
    return values or None


def runner_labels(job: dict[str, Any]) -> list[str]:
    """The labels that a job asks for, with a matrix reference expanded.

    A label that cannot be known statically stays as its source text, which is
    not a hosted label, so the job counts as self hosted.
    """
    runs_on = job.get("runs-on")
    if isinstance(runs_on, dict):
        # A runner group or a labels mapping names runners of the organization.
        return [f"group:{runs_on.get('group', '')}"]
    items = runs_on if isinstance(runs_on, list) else [runs_on]
    labels: list[str] = []
    for item in items:
        text = "" if item is None else str(item).strip()
        ref = MATRIX_REF.match(text)
        if ref:
            values = _matrix_values(job, ref.group(1))
            if values is not None:
                labels.extend(str(v).strip() for v in values)
                continue
        labels.append(text)
    return labels


def is_self_hosted(job: dict[str, Any]) -> bool:
    """True unless every label of the job is a GitHub hosted label."""
    if "runs-on" not in job:
        return False  # a job that calls a reusable workflow has no runner of its own
    labels = runner_labels(job)
    if not labels:
        return True
    return any(not HOSTED_LABEL.match(label) for label in labels)


def _contents_read_only(permissions: Any) -> str | None:
    """None when the permissions are contents read and no write, else the reason."""
    if not isinstance(permissions, dict):
        return f"permissions is {permissions!r}, not a mapping with contents: read"
    if permissions.get("contents") != "read":
        return f"contents is {permissions.get('contents')!r}, not 'read'"
    writers = sorted(str(k) for k, v in permissions.items() if v == "write")
    if writers:
        return "write permission for " + ", ".join(writers)
    return None


def _is_false(value: Any) -> bool:
    return value is False or (
        isinstance(value, str) and value.strip().lower() == "false"
    )


def _checkout_steps(job: dict[str, Any]) -> list[dict[str, Any]]:
    steps = job.get("steps")
    found: list[dict[str, Any]] = []
    if isinstance(steps, list):
        for step in steps:
            uses = step.get("uses") if isinstance(step, dict) else None
            if isinstance(uses, str) and uses.split("@", 1)[0].strip().lower() == (
                "actions/checkout"
            ):
                found.append(step)
    return found


def check_job(
    file: str, name: str, job: dict[str, Any], workflow: dict[str, Any]
) -> Iterator[Finding]:
    """Rules 2 to 4 for one self hosted job. The trigger rule is per workflow."""
    permissions = (
        job["permissions"] if "permissions" in job else workflow.get("permissions")
    )
    source = "job" if "permissions" in job else "workflow"
    problem = (
        "no permissions key at the job or the workflow"
        if permissions is None
        else _contents_read_only(permissions)
    )
    if problem:
        yield Finding(file, name, "permissions", f"{problem} (read from the {source})")

    checkouts = _checkout_steps(job)
    if not checkouts:
        yield Finding(
            file,
            name,
            "checkout",
            "no actions/checkout step with persist-credentials: false",
        )
    for step in checkouts:
        with_ = step.get("with")
        value = with_.get("persist-credentials") if isinstance(with_, dict) else None
        if not _is_false(value):
            label = step.get("name") or step.get("uses")
            yield Finding(
                file,
                name,
                "checkout",
                f"step {label!r} does not set persist-credentials: false",
            )

    timeout = job.get("timeout-minutes")
    if isinstance(timeout, bool) or not isinstance(timeout, int) or timeout <= 0:
        yield Finding(
            file,
            name,
            "timeout",
            f"timeout-minutes is {timeout!r}, not a positive whole number",
        )


def check_workflow(path: Path) -> tuple[list[Finding], list[str]]:
    """Findings and warnings for one workflow file."""
    workflow = load_workflow(path)
    file = path.name
    jobs = workflow.get("jobs")
    jobs = jobs if isinstance(jobs, dict) else {}
    self_hosted = {
        str(name): job
        for name, job in jobs.items()
        if isinstance(job, dict) and is_self_hosted(job)
    }
    findings: list[Finding] = []
    warnings: list[str] = []
    if not self_hosted:
        return findings, warnings

    extra = sorted(triggers_of(workflow) - ALLOWED_TRIGGERS)
    if extra or not triggers_of(workflow):
        names = ", ".join(extra) if extra else "none"
        findings.append(
            Finding(
                file,
                "",
                "trigger",
                f"names a self hosted runner in job(s) {', '.join(sorted(self_hosted))} "
                f"but is also started by: {names}; only workflow_dispatch and "
                "schedule are allowed, so move the job to a workflow of its own",
            )
        )
    for name, job in sorted(self_hosted.items()):
        findings.extend(check_job(file, name, job, workflow))
        guard = job.get("if")
        if not (isinstance(guard, str) and MAIN_GUARD in guard):
            warnings.append(
                f"{file}:{name}: no job if: that names {MAIN_GUARD}, so a manual run "
                "of this file on another branch would use the runner"
            )
    return findings, warnings


def check_directory(directory: Path) -> tuple[list[Finding], list[str]]:
    findings: list[Finding] = []
    warnings: list[str] = []
    paths = sorted(p for p in directory.iterdir() if p.suffix in {".yml", ".yaml"})
    for path in paths:
        more, notes = check_workflow(path)
        findings.extend(more)
        warnings.extend(notes)
    return findings, warnings


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--workflows", type=Path, default=WORKFLOWS, help="directory of workflow files"
    )
    args = parser.parse_args(argv)
    if not args.workflows.is_dir():
        print(f"error: {args.workflows} is not a directory", file=sys.stderr)
        return 2
    try:
        findings, warnings = check_directory(args.workflows)
    except WorkflowError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    for note in warnings:
        print(f"warning: {note}", file=sys.stderr)
    for finding in findings:
        print(f"error: {finding}")
    if findings:
        return 1
    print(
        "check_self_hosted: every workflow keeps self hosted jobs off untrusted triggers"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
