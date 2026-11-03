# SPDX-License-Identifier: GPL-3.0-only
"""Break the http1 request line for the negative control of the realism job.

The ``realism`` workflow runs this script on a scratch checkout, rebuilds the
http1 fixture peer, and expects the module realism job to fail. The script drops
the space between the method and the request target, so every request starts
with a line such as ``GET/a HTTP/1.1``, which nDPI does not classify as HTTP.

    python3 -m realism.ci.negative_control modules/http1/native/src/message.rs

The script exits with 2 when the renderer no longer has the expected lines, so
a change to the renderer stops the control instead of letting it pass unseen.
"""

from __future__ import annotations

import sys
from pathlib import Path

ANCHOR = "        out.extend_from_slice(self.method.as_bytes());\n        out.push(b' ');\n"
BROKEN = "        out.extend_from_slice(self.method.as_bytes());\n"


def break_request_line(source: str) -> str:
    """Return the renderer source without the space after the method."""
    count = source.count(ANCHOR)
    if count != 1:
        raise ValueError(f"expected the request line anchor once, found it {count} times")
    return source.replace(ANCHOR, BROKEN)


def main(argv: list[str] | None = None) -> int:
    """Rewrite the renderer named by the only argument and return the exit code."""
    args = sys.argv[1:] if argv is None else argv
    if len(args) != 1:
        print("usage: python3 -m realism.ci.negative_control MESSAGE_RS", file=sys.stderr)
        return 2
    path = Path(args[0])
    try:
        path.write_text(break_request_line(path.read_text(encoding="utf-8")), encoding="utf-8")
    except (OSError, ValueError) as exc:
        print(f"negative control failed: {path}: {exc}", file=sys.stderr)
        return 2
    print(f"negative control: removed the space after the method in {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
