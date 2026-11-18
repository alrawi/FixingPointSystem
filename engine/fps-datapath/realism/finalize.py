#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""The steps of the AF_XDP realism run that need what the agent reported (TRN-36).

    finalize.py counts REPO_ROOT OUT_DIR MODULE
    finalize.py checks REPO_ROOT OUT_DIR MODULE PCAP

`counts` runs after the agent and before the score. The kernel run starts an
exact number of sessions, and its labels name the numbers that follow from it
(the instances of a scenario in the structure section, the streams of the rung
check). The agent starts sessions at Poisson arrivals, so the count varies with
the seed. `counts` reads the sessions that the agent started for each scenario
from OUT_DIR/agent-N.jsonl and puts those numbers into OUT_DIR/labels.yaml. The
labels still ask for a capture that holds every session that the agent started,
so a session that never reached the wire fails the check.

`checks` runs after the score. It runs the checks that the profile of the module
names under `checks`, which are the scripts that the kernel run of the module
runs next to the score, with the numbers that follow from the sessions that the
agent started (see `expand`). A check marked `zeek` runs when Zeek is installed
and gets its path. It exits with 0 when every check passes and with the first
code that is not 0 otherwise.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from collections import Counter
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import make_plan


def started(out: Path) -> Counter[str]:
    """The sessions that the client side of the agent started, for each scenario."""
    total: Counter[str] = Counter()
    for path in sorted(out.glob("agent-*.jsonl")):
        last = None
        with path.open(encoding="utf-8") as handle:
            for line in handle:
                if line.strip():
                    last = line
        if last is None:
            continue
        for counter in json.loads(last)["counters"]:
            labels = counter["labels"]
            if counter["metric"] == "sessions.attempted" and labels["side"] == "client":
                total[labels["scenario"]] += int(counter["count"])
    return total


def apply_counts(out: Path, profile: dict[str, object]) -> None:
    counts = started(out)
    path = out / "labels.yaml"
    labels = yaml.safe_load(path.read_text(encoding="utf-8"))
    for entry in labels.get("structure", {}).get("scenarios", []):
        name = Path(entry["file"]).stem
        if "expected_instances" in entry and name in counts:
            entry["expected_instances"] = counts[name]
    streams_from = profile.get("streams_from", {})
    assert isinstance(streams_from, dict)
    rungs = labels.get("temporal", {}).get("rungs", {}).get("applications", {})
    for application, scenario in streams_from.items():
        if application in rungs and "streams" in rungs[application]:
            rungs[application]["streams"] = counts[scenario]
    header = "".join(
        line
        for line in path.read_text(encoding="utf-8").splitlines(keepends=True)
        if line.startswith("#")
    )
    path.write_text(
        header + yaml.safe_dump(labels, sort_keys=False, width=1_000_000),
        encoding="utf-8",
    )
    (out / "started.json").write_text(
        json.dumps(counts, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def expand(arg: str, values: dict[str, str], counts: Counter[str]) -> str:
    """Fills the placeholders of one argument.

    {pcap}, {view}, {zeek}, and {started} are the capture, the decrypted view,
    the Zeek program, and the sessions that the agent started. {sum:a*2+b} is
    the sum of the sessions that the agent started of the scenarios a (twice
    for each) and b, which turns the messages or transfers of a session into the
    number that the capture must hold.
    """
    for match in re.findall(r"\{sum:([^}]*)\}", arg):
        total = 0
        for term in match.split("+"):
            name, _, factor = term.partition("*")
            total += counts[name] * (int(factor) if factor else 1)
        arg = arg.replace("{sum:" + match + "}", str(total))
    return arg.format(**values)


def run_checks(
    root: Path, out: Path, module: str, profile: dict[str, object], pcap: Path
) -> int:
    counts = started(out)
    zeek = os.environ.get("FPS_ZEEK") or shutil.which("zeek")
    if zeek is None and Path("/opt/zeek/bin/zeek").is_file():
        zeek = "/opt/zeek/bin/zeek"
    values = {
        "pcap": str(pcap),
        "view": str(out / f"{module}-afxdp.decrypted.pcap"),
        "started": str(sum(counts.values())),
        "zeek": zeek or "",
    }
    checks = profile.get("checks", [])
    assert isinstance(checks, list)
    rc = 0
    for check in checks:
        if check.get("zeek") and zeek is None:
            # The kernel run skips these checks the same way, and the realism
            # job installs Zeek.
            print(f"finalize.py: skipped {check['run'][0]}, Zeek is not installed", file=sys.stderr)
            continue
        argv = [expand(str(arg), values, counts) for arg in check["run"]]
        if check.get("zeek"):
            argv += ["--zeek", values["zeek"]]
        print("finalize.py: " + " ".join(argv), file=sys.stderr)
        proc = subprocess.run(["python3", *argv], cwd=root, check=False)
        if proc.returncode != 0 and rc == 0:
            rc = proc.returncode
    return rc


def main(argv: list[str]) -> int:
    if len(argv) < 4 or argv[0] not in ("counts", "checks"):
        print(__doc__, file=sys.stderr)
        return 2
    root, out, module = Path(argv[1]).resolve(), Path(argv[2]).resolve(), argv[3]
    profile = make_plan.load_profile(module)
    if argv[0] == "counts":
        apply_counts(out, profile)
        return 0
    if len(argv) != 5:
        print(__doc__, file=sys.stderr)
        return 2
    return run_checks(root, out, module, profile, Path(argv[4]).resolve())


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
