# SPDX-License-Identifier: GPL-3.0-only
"""The capture of netns.sh keeps a burst that arrives while tcpdump is not running.

Bug 735 found that tcpdump with -s 0 and the default 2 MB buffer holds about 30
frames in immediate mode, so a tcpdump that a busy runner did not schedule for a
few milliseconds lost the rest of a burst without a word. Bug 738 applies the fix
to capture_start and makes capture_stop fail when tcpdump reports drops. The
end-to-end test needs root, ip, and tcpdump and skips elsewhere. The stub tests
need only root-free tools and check that capture_stop fails on a drop count.
"""

from __future__ import annotations

import os
import re
import shutil
import signal
import subprocess
import time
from pathlib import Path

import pytest

NETNS = Path(__file__).resolve().parent / "netns.sh"
BURST = 6000

SEND = (
    "import socket, sys, time\n"
    "s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)\n"
    "dst = (sys.argv[1], 9)\n"
    "s.sendto(b'w', dst)\n"  # the warm-up resolves ARP before the burst
    "time.sleep(0.5)\n"
    f"[s.sendto(b'x' * 1200, dst) for _ in range({BURST})]\n"
)

needs_root = pytest.mark.skipif(
    os.geteuid() != 0
    or not shutil.which("ip")
    or not shutil.which("tcpdump")
    or not shutil.which("setsid"),
    reason="needs root, ip, setsid, and tcpdump",
)


def netns(env: dict[str, str], *args: str, check: bool = True) -> str:
    done = subprocess.run(
        [str(NETNS), *args],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if check and done.returncode != 0:
        raise AssertionError(f"netns.sh {' '.join(args)}: {done.stderr}")
    return done.stdout + done.stderr if not check else done.stdout


def harness_env(tmp_path: Path, prefix: str, path: str | None = None) -> dict[str, str]:
    env = dict(os.environ, FPS_NETNS=prefix, FPS_STATE_DIR=str(tmp_path / "state"))
    if path is not None:
        env["PATH"] = path
    return env


def info(env: dict[str, str], key: str) -> str:
    match = re.search(rf"^{key}=(.*)$", netns(env, "info"), re.MULTILINE)
    assert match, key
    return match.group(1)


@needs_root
def test_a_stopped_tcpdump_loses_no_frame_of_a_burst(tmp_path: Path) -> None:
    env = harness_env(tmp_path, "fps738a")
    netns(env, "down")
    netns(env, "up")
    try:
        out = tmp_path / "burst.pcapng"
        netns(env, "capture", "start", str(out), "client")
        pid = int((tmp_path / "state" / "fps738a.capture.pid").read_text())
        # setsid and ip netns exec replace themselves, so the pid is the tcpdump.
        os.kill(pid, signal.SIGSTOP)
        netns(
            env, "exec", "client", "python3", "-I", "-c", SEND, info(env, "SERVER_ADDR")
        )
        time.sleep(0.2)
        os.kill(pid, signal.SIGCONT)
        time.sleep(0.5)
        netns(env, "capture", "stop")
        assert out.is_file()
        log = (tmp_path / "state" / "fps738a.capture.log").read_text()
        match = re.search(r"(\d+) packets? dropped by kernel", log)
        assert match, log
        assert int(match.group(1)) == 0, log
    finally:
        netns(env, "down", check=False)


def stub_tcpdump(directory: Path) -> str:
    """A tcpdump that listens, writes an empty pcap, and prints statistics on SIGINT."""
    directory.mkdir()
    stub = directory / "tcpdump"
    stub.write_text(
        "#!/usr/bin/env python3\n"
        "import os, signal, struct, sys, time\n"
        "raw = sys.argv[sys.argv.index('-w') + 1]\n"
        "def stop(*_):\n"
        "    sys.stderr.write(os.environ['STUB_STATS'])\n"
        "    sys.stderr.flush()\n"
        "    os._exit(0)\n"
        # A background job of a script starts with SIGINT ignored, as tcpdump resets.
        "signal.signal(signal.SIGINT, stop)\n"
        "open(raw, 'wb').write(struct.pack('<IHHiIII', 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1))\n"
        "sys.stderr.write('listening on any\\n')\n"
        "sys.stderr.flush()\n"
        "time.sleep(60)\n",
        encoding="utf-8",
    )
    stub.chmod(0o755)
    return str(directory)


@needs_root
@pytest.mark.parametrize(
    ("statistics", "message"),
    [
        (
            "0 packets captured\n7 packets dropped by kernel\n",
            "tcpdump dropped 7 packets",
        ),
        ("0 packets captured\n", "did not report its drop count"),
    ],
)
def test_capture_stop_fails_when_tcpdump_drops_or_reports_nothing(
    tmp_path: Path, statistics: str, message: str
) -> None:
    stub_dir = stub_tcpdump(tmp_path / "bin")
    env = harness_env(tmp_path, "fps738b", f"{stub_dir}:{os.environ['PATH']}")
    env["STUB_STATS"] = statistics
    netns(env, "down")
    netns(env, "up")
    try:
        out = tmp_path / "x.pcapng"
        netns(env, "capture", "start", str(out), "client")
        done = subprocess.run(
            [str(NETNS), "capture", "stop"],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        assert done.returncode != 0, done.stderr
        assert message in done.stderr, done.stderr
        assert not out.exists()
    finally:
        netns(env, "down", check=False)


def test_capture_start_uses_a_snaplen_near_the_frame_and_a_large_buffer() -> None:
    text = NETNS.read_text(encoding="utf-8")
    line = next(l for l in text.splitlines() if "tcpdump -n " in l)
    assert "-s 9216" in line
    assert "-B 65536" in line
