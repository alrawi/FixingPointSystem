# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of zeek-gate2.sh and of its use in mix.sh, which need bash and no Zeek."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[2]))

WRAPPER = HERE / "zeek-gate2.sh"
QUEUE = "max_event_queue_size=0"

# A stand in for Zeek. It records its arguments, and when it reads a capture it writes the
# logs into the working directory like Zeek. A weird of a connection is always there. The
# weird of the event queue, which has no connection, is there only when the queue limit
# is still on, like the real Zeek that drains its queue at the end of a large trace.
FAKE_ZEEK = """#!/usr/bin/env bash
printf '%s\\n' "$@" > "$FAKE_ZEEK_ARGS"
case " $* " in
  *" --version "*) echo "zeek version 9.0.1"; exit 0 ;;
esac
echo '{"ts":10.5,"uid":"C1","id.orig_h":"10.201.0.2","id.orig_p":41000,"id.resp_h":"10.202.0.2","id.resp_p":8082,"name":"bad_HTTP_request","notice":false,"peer":"zeek"}' > weird.log
case " $* " in
  *" max_event_queue_size=0 "*) ;;
  *) echo '{"ts":141.023,"name":"event_queue_overflow_drain","notice":false,"peer":"zeek"}' >> weird.log ;;
esac
echo '{"ts":10.4,"uid":"C1","id.orig_h":"10.201.0.2","id.orig_p":41000,"id.resp_h":"10.202.0.2","id.resp_p":8082,"proto":"tcp"}' > conn.log
"""


@pytest.fixture
def fake_zeek(tmp_path: Path) -> Path:
    path = tmp_path / "fake-zeek"
    path.write_text(FAKE_ZEEK, encoding="utf-8")
    path.chmod(0o755)
    return path


def run_wrapper(
    tmp_path: Path, fake: Path | None, *args: str, search_path: str = "/usr/bin:/bin"
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    record = tmp_path / "args.txt"
    env = {
        "PATH": search_path,
        "FAKE_ZEEK_ARGS": str(record),
        "HOME": str(tmp_path),
    }
    if fake is not None:
        env["FPS_GATE2_ZEEK"] = str(fake)
    work = tmp_path / "work"
    work.mkdir(exist_ok=True)
    proc = subprocess.run(
        [shutil.which("bash") or "/bin/bash", str(WRAPPER), *args],
        capture_output=True,
        text=True,
        env=env,
        cwd=work,
        check=False,
    )
    seen = record.read_text(encoding="utf-8").splitlines() if record.exists() else []
    return proc, seen


def test_reading_a_capture_switches_the_queue_limit_off(
    tmp_path: Path, fake_zeek: Path
) -> None:
    argv = ["-C", "-r", "/x/mix.pcapng", "LogAscii::use_json=T"]
    proc, seen = run_wrapper(tmp_path, fake_zeek, *argv)
    assert proc.returncode == 0, proc.stderr
    assert seen == [*argv, QUEUE]


def test_other_calls_pass_unchanged(tmp_path: Path, fake_zeek: Path) -> None:
    proc, seen = run_wrapper(tmp_path, fake_zeek, "--version")
    assert proc.returncode == 0
    assert "zeek version 9.0.1" in proc.stdout
    assert seen == ["--version"]


def test_the_exit_code_of_zeek_is_the_exit_code_of_the_script(tmp_path: Path) -> None:
    failing = tmp_path / "failing-zeek"
    failing.write_text("#!/usr/bin/env bash\nexit 3\n", encoding="utf-8")
    failing.chmod(0o755)
    proc, _ = run_wrapper(tmp_path, failing, "-r", "x.pcap")
    assert proc.returncode == 3


def test_a_missing_zeek_fails_with_127(tmp_path: Path) -> None:
    if Path("/opt/zeek/bin/zeek").exists():
        pytest.skip("this machine has Zeek in /opt/zeek/bin")
    only_bash = tmp_path / "only-bash"
    only_bash.mkdir()
    (only_bash / "bash").symlink_to(shutil.which("bash") or "/bin/bash")
    proc, _ = run_wrapper(tmp_path, None, "-r", "x.pcap", search_path=str(only_bash))
    assert proc.returncode == 127
    assert "FPS_GATE2_ZEEK" in proc.stderr


def test_zeek_on_the_search_path_is_found(tmp_path: Path, fake_zeek: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "zeek").symlink_to(fake_zeek)
    path = f"{bin_dir}:/usr/bin:/bin"
    proc, seen = run_wrapper(tmp_path, None, "-r", "x.pcap", search_path=path)
    assert proc.returncode == 0, proc.stderr
    assert seen == ["-r", "x.pcap", QUEUE]


def test_the_runner_keeps_the_weird_of_a_session_and_loses_the_queue_weird(
    tmp_path: Path, fake_zeek: Path
) -> None:
    """Through the wrapper, the weird of a session is a finding and the queue weird is gone.

    Without the wrapper the stand in raises the queue weird, which has no session, so the
    test also shows the failure that the wrapper prevents.
    """
    parsers = pytest.importorskip("realism.parsers")
    capture = tmp_path / "mix.pcapng"
    capture.write_bytes(b"")
    os.environ["FAKE_ZEEK_ARGS"] = str(tmp_path / "runner-args.txt")
    try:
        bare = parsers.ZeekRunner(program=str(fake_zeek)).findings(capture)
        os.environ["FPS_GATE2_ZEEK"] = str(fake_zeek)
        wrapped = parsers.ZeekRunner(program=str(WRAPPER)).findings(capture)
    finally:
        os.environ.pop("FPS_GATE2_ZEEK", None)
        os.environ.pop("FAKE_ZEEK_ARGS", None)
    assert sorted(f.name for f in bare) == [
        "bad_HTTP_request",
        "event_queue_overflow_drain",
    ]
    assert [f.name for f in wrapped] == ["bad_HTTP_request"]
    assert wrapped[0].key is not None
    assert wrapped[0].dst_port == 8082


def test_mix_sh_runs_the_realism_runner_through_the_wrapper() -> None:
    text = (HERE / "mix.sh").read_text(encoding="utf-8")
    assert 'export FPS_ZEEK="$here/zeek-gate2.sh"' in text
    assert "export FPS_GATE2_ZEEK=$zeek_real" in text
    assert os.access(WRAPPER, os.X_OK)
