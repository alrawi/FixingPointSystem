#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Runs a newer ndpiReader so that the realism runner can read its flow CSV.

nDPI 4.2.0 has no HTTP/2 protocol, so scoring the http2 fixture needs a newer
ndpiReader. Releases after 4.2 write the flow file of ``-C`` with ``|`` as the
separator, and the realism runner reads commas. This wrapper passes its
arguments to the reader that ``FPS_NDPI_REAL_READER`` names and rewrites the
``-C`` file with commas and quoting where a field needs it. Select it with

    FPS_NDPI_REAL_READER=/path/to/ndpiReader \\
    FPS_NDPI_READER=modules/http2/fixtures/ndpi_reader_compat.py

It exits with the code of the reader, except for ``-h``, which newer readers end
with 1 after the usage text and which the runner calls to read the version.
"""

from __future__ import annotations

import csv
import io
import os
import subprocess
import sys
from pathlib import Path


def main(argv: list[str]) -> int:
    reader = os.environ.get("FPS_NDPI_REAL_READER", "ndpiReader")
    try:
        proc = subprocess.run([reader, *argv], check=False)
    except OSError as exc:
        print(f"ndpi_reader_compat: {reader}: {exc.strerror}", file=sys.stderr)
        return 127
    if argv in (['-h'], ['-H']):
        return 0
    if "-C" in argv[:-1]:
        out = Path(argv[argv.index("-C") + 1])
        try:
            text = out.read_text(encoding="utf-8", errors="replace")
        except OSError:
            return proc.returncode
        first = text.splitlines()[0] if text else ""
        if "|" in first:
            buffer = io.StringIO()
            writer = csv.writer(buffer, lineterminator="\n")
            for row in csv.reader(text.splitlines(), delimiter="|", quoting=csv.QUOTE_NONE):
                writer.writerow(row)
            out.write_text(buffer.getvalue(), encoding="utf-8")
    return proc.returncode


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
