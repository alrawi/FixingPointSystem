# SPDX-License-Identifier: GPL-3.0-only
"""The capture of module-afxdp.sh keeps a burst that arrives while tcpdump is not running.

Bug 735: tcpdump ran with -s 0 and the default buffer, whose ring holds about 30
frames in immediate mode. A tcpdump that a busy runner did not schedule for a few
milliseconds lost the rest of one TLS session, and the encryption check then read
79 of 80 sessions. The test stops the tcpdump of the script, sends a burst, and
needs root, ip, and tcpdump, so it skips elsewhere.
"""

from __future__ import annotations

import os
import re
import shlex
import shutil
import signal
import subprocess
import time
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parent / "module-afxdp.sh"
BURST = 200

SEND = (
    "import socket\n"
    "s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)\n"
    "s.bind(('10.97.0.1', 0))\n"
    f"[s.sendto(b'x' * 1200, ('10.97.0.2', 9)) for _ in range({BURST})]\n"
)


def capture_options() -> list[str]:
    """The tcpdump options of the script, without the interface and the file."""
    line = next(
        l
        for l in SCRIPT.read_text(encoding="utf-8").splitlines()
        if " tcpdump -n " in l
    )
    words = shlex.split(line.split(" tcpdump ", 1)[1])
    return words[: words.index("-i")]


def test_the_script_checks_the_drop_count_of_tcpdump() -> None:
    text = SCRIPT.read_text(encoding="utf-8")
    assert "packets dropped by kernel" in text or "dropped by" in text
    assert re.search(r"dropped\s+\$dropped packets", text)


@pytest.mark.skipif(
    os.geteuid() != 0 or not shutil.which("ip") or not shutil.which("tcpdump"),
    reason="needs root, ip, and tcpdump",
)
def test_a_stopped_tcpdump_loses_no_frame_of_a_burst(tmp_path: Path) -> None:
    names = ("fps735a", "fps735b")
    for name in names:
        subprocess.run(["ip", "netns", "del", name], capture_output=True)
    try:
        for name in names:
            subprocess.run(["ip", "netns", "add", name], check=True)
        subprocess.run(
            [
                "ip",
                "-n",
                names[0],
                "link",
                "add",
                "va",
                "type",
                "veth",
                "peer",
                "name",
                "vb",
                "netns",
                names[1],
            ],
            check=True,
        )
        for ns, dev, addr in (
            (names[0], "va", "10.97.0.1/24"),
            (names[1], "vb", "10.97.0.2/24"),
        ):
            subprocess.run(
                ["ip", "-n", ns, "addr", "add", addr, "dev", dev], check=True
            )
            subprocess.run(["ip", "-n", ns, "link", "set", dev, "up"], check=True)
        subprocess.run(
            [
                "ip",
                "-n",
                names[0],
                "neigh",
                "add",
                "10.97.0.2",
                "lladdr",
                "02:00:00:00:00:02",
                "dev",
                "va",
                "nud",
                "permanent",
            ],
            check=True,
        )
        log = tmp_path / "tcpdump.log"
        with log.open("w") as err:
            dump = subprocess.Popen(
                [
                    "ip",
                    "netns",
                    "exec",
                    names[1],
                    "tcpdump",
                    "-n",
                    *capture_options(),
                    "-i",
                    "vb",
                    "-w",
                    str(tmp_path / "x.pcap"),
                ],
                stderr=err,
            )
        for _ in range(50):
            if "listening on" in log.read_text():
                break
            time.sleep(0.1)
        # ip netns exec replaces itself with tcpdump, so the pid is the tcpdump.
        os.kill(dump.pid, signal.SIGSTOP)
        subprocess.run(
            ["ip", "netns", "exec", names[0], "python3", "-I", "-c", SEND], check=True
        )
        time.sleep(0.2)
        os.kill(dump.pid, signal.SIGCONT)
        time.sleep(0.5)
        dump.terminate()
        dump.wait(timeout=10)
        stats = log.read_text()
        match = re.search(r"(\d+) packets? dropped by kernel", stats)
        assert match, stats
        assert int(match.group(1)) == 0, stats
    finally:
        for name in names:
            subprocess.run(["ip", "netns", "del", name], capture_output=True)
