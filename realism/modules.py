# SPDX-License-Identifier: GPL-3.0-only
"""Selection of the native modules that the module realism job scores.

A native module takes part when its directory under ``modules/`` holds a native
crate in ``native/Cargo.toml``, a harness capture script in
``fixtures/capture.sh``, and expected labels in ``fixtures/expected.yaml``. The
``realism`` workflow lists the changed paths of a pull request, and
:func:`select` keeps the modules that those paths touch. A change to a shared
path, such as the realism runner, the engine, or the harness, selects every
module, because it can change the traffic or the score of each of them.
"""

from __future__ import annotations

import re
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path

MODULES_DIR = "modules"

# Paths whose change selects every module. A prefix that ends in a slash names
# a directory, and any other entry names one file.
SHARED_PATHS = (
    "realism/",
    "engine/",
    "tests/harness/",
    "Cargo.toml",
    "Cargo.lock",
    "mk/rlm.mk",
    ".github/workflows/realism.yml",
)

_NAME = re.compile(r"[a-z0-9][a-z0-9_-]*")
_SECTION = re.compile(r"\s*\[([^\]]*)\]\s*(#.*)?")
_CRATE = re.compile(r'\s*name\s*=\s*"([A-Za-z0-9_-]+)"\s*(#.*)?')


class ModuleError(Exception):
    """A module directory looks like a native module but cannot be read."""


@dataclass(frozen=True)
class Module:
    """One native module with the files that the realism job needs."""

    name: str
    crate: str

    def to_json(self) -> dict[str, str]:
        """Return the matrix entry of the module."""
        return {"module": self.name, "crate": self.crate}


def crate_name(manifest: Path) -> str:
    """Return the package name of a Cargo manifest.

    The parser reads only the ``name`` key of the ``[package]`` table, which
    keeps the runner on the standard library of Python 3.10.
    """
    try:
        lines = manifest.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        raise ModuleError(f"read {manifest}: {exc.strerror}") from exc
    section = ""
    for line in lines:
        header = _SECTION.fullmatch(line)
        if header is not None:
            section = header.group(1).strip()
            continue
        if section == "package":
            name = _CRATE.fullmatch(line)
            if name is not None:
                return name.group(1)
    raise ModuleError(f"{manifest} has no package name")


def discover(root: Path) -> list[Module]:
    """Return the native modules under ``root/modules``, sorted by name.

    Directories whose name starts with an underscore, such as ``_template``, and
    directories without the three files are not modules of this job.
    """
    base = root / MODULES_DIR
    if not base.is_dir():
        return []
    found = []
    for path in sorted(base.iterdir()):
        if not path.is_dir() or not _NAME.fullmatch(path.name):
            continue
        manifest = path / "native" / "Cargo.toml"
        needed = (manifest, path / "fixtures" / "capture.sh", path / "fixtures" / "expected.yaml")
        if not all(item.is_file() for item in needed):
            continue
        found.append(Module(name=path.name, crate=crate_name(manifest)))
    return found


def _shared(path: str) -> bool:
    return any(
        path.startswith(entry) if entry.endswith("/") else path == entry for entry in SHARED_PATHS
    )


def select(modules: Iterable[Module], changed: Iterable[str] | None) -> list[Module]:
    """Return the modules that the changed paths touch.

    ``changed`` holds paths relative to the repository root, as ``git diff
    --name-only`` prints them. None means that the change set is unknown, as on
    a push to ``main``, and selects every module, and so does a shared path.
    """
    candidates = list(modules)
    if changed is None:
        return candidates
    paths = [line.strip() for line in changed if line.strip()]
    if any(_shared(path) for path in paths):
        return candidates
    touched = {
        parts[1]
        for parts in (path.split("/") for path in paths)
        if len(parts) > 2 and parts[0] == MODULES_DIR
    }
    return [module for module in candidates if module.name in touched]


def matrix(modules: Iterable[Module]) -> dict[str, list[dict[str, str]]]:
    """Return a GitHub Actions matrix with one entry per module."""
    return {"include": [module.to_json() for module in modules]}
