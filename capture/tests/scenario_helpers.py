# SPDX-License-Identifier: GPL-3.0-only
"""Helpers for the scenario tests: synthetic sessions, generation, and the contract validator."""

from __future__ import annotations

import base64
import importlib.util
import json
import sys
from pathlib import Path
from types import ModuleType
from typing import Any

import yaml

from capture.cli import extract, main

from .pcapgen import ACK, FIN, PSH, SYN, Rec, tcp_frame, udp_frame, write_pcap

C, S = "192.0.2.10", "192.0.2.80"
T0 = 1_700_000_000.0  # 2023-11-14 22:13:20 UTC, so that dates and epochs in tests are "now"
REPO = Path(__file__).resolve().parents[2]


def tcp_session(
    turns: list[tuple[str, bytes]],
    t0: float = T0,
    *,
    sport: int = 40000,
    dport: int = 8080,
    pause: float = 0.01,
    client: str = C,
    server: str = S,
) -> list[Rec]:
    """Records of one TCP connection: handshake, one segment per turn, and a FIN close.

    A turn is ``("c", bytes)`` for the client or ``("s", bytes)`` for the server."""
    recs: list[Rec] = []
    t = t0
    cs, ss = 1000 + sport, 5000 + sport
    recs.append(Rec(t, tcp_frame(client, server, sport, dport, seq=cs, flags=SYN)))
    recs.append(
        Rec(t + 0.001, tcp_frame(server, client, dport, sport, seq=ss, ack=cs + 1, flags=SYN | ACK))
    )
    recs.append(
        Rec(t + 0.002, tcp_frame(client, server, sport, dport, seq=cs + 1, ack=ss + 1, flags=ACK))
    )
    t += 0.003
    co, so = cs + 1, ss + 1
    for who, data in turns:
        t += pause
        if who == "c":
            frame = tcp_frame(
                client, server, sport, dport, seq=co, ack=so, flags=ACK | PSH, payload=data
            )
            co += len(data)
        else:
            frame = tcp_frame(
                server, client, dport, sport, seq=so, ack=co, flags=ACK | PSH, payload=data
            )
            so += len(data)
        recs.append(Rec(t, frame))
    recs.append(
        Rec(t + 0.001, tcp_frame(client, server, sport, dport, seq=co, ack=so, flags=FIN | ACK))
    )
    recs.append(
        Rec(t + 0.002, tcp_frame(server, client, dport, sport, seq=so, ack=co + 1, flags=FIN | ACK))
    )
    recs.append(
        Rec(t + 0.003, tcp_frame(client, server, sport, dport, seq=co + 1, ack=so + 1, flags=ACK))
    )
    return recs


def udp_session(
    datagrams: list[tuple[str, bytes]],
    t0: float = T0,
    *,
    sport: int = 50000,
    dport: int = 5353,
    client: str = C,
    server: str = S,
) -> list[Rec]:
    """Records of one UDP flow."""
    recs = []
    for n, (who, data) in enumerate(datagrams):
        if who == "c":
            frame = udp_frame(client, server, sport, dport, data)
        else:
            frame = udp_frame(server, client, dport, sport, data)
        recs.append(Rec(t0 + 0.01 * n, frame))
    return recs


def sessions_doc(tmp_path: Path, recs: list[Rec]) -> tuple[dict[str, Any], Path]:
    """Write a capture, extract its sessions with payloads, and save the document."""
    pcap = tmp_path / "in.pcap"
    pcap.write_bytes(write_pcap(sorted(recs, key=lambda r: r.t)))
    doc = extract(pcap, payloads=True)
    path = tmp_path / "sessions.json"
    path.write_text(json.dumps(doc), encoding="utf-8")
    return doc, path


def generate(
    tmp_path: Path, path: Path, *args: str, module: str = "gen", synthetic: bool = True
) -> Path:
    """Run ``python3 -m capture scenario`` and return the module directory."""
    out = tmp_path / f"parent_{module}" / module  # one module per parent for the validator
    argv = ["scenario", "--sessions", str(path), "--out-dir", str(out), *args]
    if "--session" not in args:
        argv.append("--all")
    if synthetic:
        argv.append("--synthetic")
    assert main(argv) == 0
    return out


def load_validator() -> ModuleType:
    """Import ``tools/contracts/validate.py``, which is not a package."""
    spec = importlib.util.spec_from_file_location(
        "fps_contract_validate", REPO / "tools" / "contracts" / "validate.py"
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def validation_errors(module_dir: Path) -> list[str]:
    """Findings of severity error that the contract validator reports for the module."""
    validator = load_validator()
    report = validator.run([module_dir.parent], [], None)
    return [str(f) for f in report.errors]


def scenario_docs(module_dir: Path) -> dict[str, dict[str, Any]]:
    """The scenario documents of a module, by file stem."""
    return {
        p.stem: yaml.safe_load(p.read_text(encoding="utf-8"))
        for p in sorted((module_dir / "scenarios").glob("*.yaml"))
    }


def stream_bytes(session: dict[str, Any], direction: str) -> bytes:
    """The payload of one direction of a TCP session, from the document."""
    return b"".join(
        base64.b64decode(t["data_b64"]) for t in session["turns"] if t["direction"] == direction
    )
