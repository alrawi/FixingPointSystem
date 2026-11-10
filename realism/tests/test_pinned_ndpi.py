# SPDX-License-Identifier: GPL-3.0-only
"""The nDPI pin of the CI workflows lives in one file and the script that builds it."""

from __future__ import annotations

import os
import re
import stat
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CI = ROOT / "realism" / "ci"
PIN = CI / "ndpi.version"
BUILD = CI / "build-ndpi.sh"
INSTALL = CI / "install-tools.sh"
WORKFLOWS = [ROOT / ".github" / "workflows" / name for name in ("rlm.yml", "realism.yml")]


def test_the_pin_is_a_release_and_the_commit_of_its_tag() -> None:
    assert re.fullmatch(r"\d+\.\d+(\.\d+)? [0-9a-f]{40}\n", PIN.read_text(encoding="utf-8"))


def test_the_pin_asks_for_a_release_with_http2() -> None:
    major, minor, *_ = PIN.read_text(encoding="utf-8").split()[0].split(".")
    assert (int(major), int(minor)) >= (4, 14)


def test_the_build_script_reads_the_pin_and_checks_the_commit() -> None:
    text = BUILD.read_text(encoding="utf-8")
    assert "ndpi.version" in text
    assert 'fetch -q --depth 1 https://github.com/ntop/nDPI.git "$commit"' in text
    assert "rev-parse HEAD" in text
    assert os.stat(BUILD).st_mode & stat.S_IXUSR
    assert not re.search(r"\b4\.\d+\.\d+\b", re.sub(r"#.*", "", text))


def test_the_install_script_runs_the_build_after_the_apt_packages() -> None:
    text = INSTALL.read_text(encoding="utf-8")
    assert text.rstrip().endswith('"$here/build-ndpi.sh"')
    assert text.index("tools/ci/apt-install.sh") < text.rindex("build-ndpi.sh")


def test_both_workflows_install_through_the_script_and_name_no_ndpi_version() -> None:
    for workflow in WORKFLOWS:
        text = workflow.read_text(encoding="utf-8")
        assert "realism/ci/install-tools.sh" in text, workflow.name
        assert "ndpi.version" not in text and "build-ndpi" not in text, workflow.name


def test_a_malformed_pin_fails_before_anything_is_installed(tmp_path: Path) -> None:
    for pin in ("4.14", "4.14 abc", "main 90090b9ae841fce67d00c52ac87e755dcea7c95f"):
        proc = subprocess.run(
            ["bash", str(BUILD)],
            env={"PATH": os.environ["PATH"], "FPS_NDPI_VERSION": pin, "HOME": str(tmp_path)},
            capture_output=True,
            text=True,
            check=False,
        )
        assert proc.returncode == 2, (pin, proc.stderr)
        assert "is not a release and a 40 digit commit id" in proc.stderr


def test_a_reader_with_the_pinned_release_needs_no_build(tmp_path: Path) -> None:
    reader = tmp_path / "bin" / "ndpiReader"
    reader.parent.mkdir()
    reader.write_text("#!/bin/sh\necho 'Welcome to nDPI 4.14.0-4875-abc1234'\necho usage\nexit 1\n")
    reader.chmod(0o755)
    sudo = tmp_path / "sudo"
    sudo.write_text('#!/bin/sh\necho "sudo $*" >> "$FPS_SUDO_LOG"\n')
    sudo.chmod(0o755)
    log = tmp_path / "sudo.log"
    env = {
        "PATH": f"{tmp_path}:{os.environ['PATH']}",
        "FPS_NDPI_PREFIX": str(tmp_path),
        "FPS_SUDO_LOG": str(log),
        "HOME": str(tmp_path),
    }
    proc = subprocess.run(
        ["bash", str(BUILD)], env=env, capture_output=True, text=True, check=False
    )
    assert proc.returncode == 0, proc.stderr
    assert "Welcome to nDPI 4.14.0" in proc.stdout
    # Only the link into /usr/local/bin ran with sudo: no apt-get, no build.
    assert log.read_text().splitlines() == [f"sudo ln -sf {reader} /usr/local/bin/ndpiReader"]


def test_a_reader_of_another_release_is_not_accepted(tmp_path: Path) -> None:
    reader = tmp_path / "bin" / "ndpiReader"
    reader.parent.mkdir()
    reader.write_text("#!/bin/sh\necho 'Welcome to nDPI 4.2.0-3789'\nexit 1\n")
    reader.chmod(0o755)
    sudo = tmp_path / "sudo"
    # A sudo that fails stops the script where it would install the build dependencies.
    sudo.write_text("#!/bin/sh\nexit 77\n")
    sudo.chmod(0o755)
    env = {
        "PATH": f"{tmp_path}:{os.environ['PATH']}",
        "FPS_NDPI_PREFIX": str(tmp_path),
        "HOME": str(tmp_path),
    }
    proc = subprocess.run(
        ["bash", str(BUILD)], env=env, capture_output=True, text=True, check=False
    )
    assert proc.returncode == 77
