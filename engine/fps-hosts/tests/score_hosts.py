# SPDX-License-Identifier: GPL-3.0-only
"""Score a capture with the host identity check of the realism runner.

Usage: score_hosts.py CAPTURE --hosts N [--macs N] [--expect-conflicts]

The script runs ``realism.hosts.check`` on the capture and prints the result as JSON. It exits
with 0 when the check passes and the counts match, and with 1 otherwise. With
``--expect-conflicts`` it inverts the verdict on conflicts: it exits with 0 only when the check
fails and reports at least one address conflict, which proves that the check sees a duplicate
address. The integration tests of ``engine/fps-hosts`` call it as a plain argv.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from realism.hosts import check  # noqa: E402
from realism.labels import Count  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--hosts", type=int, required=True, help="exact number of hosts expected")
    parser.add_argument("--macs", type=int, help="exact number of MAC addresses expected")
    parser.add_argument("--expect-conflicts", action="store_true")
    args = parser.parse_args()

    result = check(args.capture, expected=Count(args.hosts, args.hosts))
    report = result.to_dict()
    summary = {
        key: report[key]
        for key in ("passed", "hosts", "senders", "macs", "frames", "conflicts_total", "failures")
    }
    print(json.dumps(summary))
    if args.expect_conflicts:
        return 0 if not result.passed and result.conflicts else 1
    if not result.passed:
        return 1
    if args.macs is not None and result.macs != args.macs:
        print(f"expected {args.macs} MAC addresses, found {result.macs}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
