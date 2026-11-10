# SPDX-License-Identifier: GPL-3.0-only
"""Replay generated scenarios with the real agent and compare byte counts with the capture.

The check behind the acceptance criterion of CAP-02: a scenario that the generator wrote
validates and replays with the same byte counts within 1%.

1. Build a synthetic capture with an HTTP session (Content-Length, cookie, date, request
   id, epoch time), a TLS style framed session, a line protocol session, and a UDP flow,
   each on its own server port.
2. Extract the sessions, generate a module, and write an agent plan that runs every
   scenario at a low rate between two host pools.
3. Run the plan with ``fps-agent run`` and record the traffic with tcpdump.
4. Extract the sessions of the replay capture and compare each replay session with the
   captured session of the same server port, direction by direction.

Two modes run it. ``netns`` uses the QA-01 namespace harness (``tests/harness/netns.sh``)
with the server side in the server namespace and the client side in the client namespace,
which needs root, ``ip``, and tcpdump, and which CI provides. ``loopback`` runs both sides
in one process on 127.0.0.0/8 and records the ``lo`` interface, which needs root and tcpdump
but no ``ip`` (the pools are loopback addresses, so the sandbox without ``ip`` can run it).

    python3 -m capture.tests.replay --mode netns --agent target/debug/fps-agent
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

from capture.cli import extract, main

from .pcapgen import Rec, write_pcap
from .scenario_helpers import REPO, T0, tcp_session, udp_session

MODULE = "cap02replay"
HARNESS = REPO / "tests" / "harness" / "netns.sh"
EXAMPLE_PLAN = REPO / "contracts" / "examples" / "plans" / "http_dns_smoke.json"

BODY = ("<html>" + "lorem ipsum dolor sit amet " * 60 + "</html>").encode()
HTTP_TURNS = [
    (
        "c",
        b"GET /index.html HTTP/1.1\r\nHost: www.example.test\r\n"
        b"X-Request-Id: 7f3a9c2e41b8d605\r\nAccept: */*\r\n\r\n",
    ),
    (
        "s",
        b"HTTP/1.1 200 OK\r\nDate: Tue, 14 Nov 2023 22:13:25 GMT\r\n"
        b"Set-Cookie: sid=9d41c7e2b0a35f68; Path=/\r\nX-Request-Id: 7f3a9c2e41b8d605\r\n"
        b"Content-Length: %d\r\n\r\n" % len(BODY) + BODY,
    ),
    (
        "c",
        b"GET /next HTTP/1.1\r\nHost: www.example.test\r\nCookie: lang=en; sid=9d41c7e2b0a35f68\r\n"
        b"X-Time: 1700000012\r\n\r\n",
    ),
    (
        "s",
        b"HTTP/1.1 204 No Content\r\nDate: Tue, 14 Nov 2023 22:13:25 GMT\r\n"
        b"Content-Length: 0\r\n\r\n",
    ),
]
SMTP_TURNS = [
    ("s", b"220 mail.example.test ready\r\n"),
    ("c", b"EHLO client.example.test\r\n"),
    ("s", b"250-mail.example.test\r\n250 PIPELINING\r\n"),
    ("c", b"QUIT\r\n"),
    ("s", b"221 bye\r\n"),
]
PAYLOAD = bytes((i * 7 + 3) % 256 for i in range(1500))
FRAMED = b"\x17\x03\x03" + struct.pack(">H", len(PAYLOAD)) + PAYLOAD
HELLO = b"\x16\x03\x01\x00\x10" + bytes(range(16))
FRAME_TURNS = [("c", HELLO), ("s", HELLO), ("c", FRAMED), ("s", FRAMED + FRAMED)]
UDP_TURNS = [
    ("c", b"QUERY a91f03c7d2e5 host.example.test\r\n"),
    ("s", b"ANSWER a91f03c7d2e5 192.0.2.1\r\n"),
    ("c", b"PING\r\n"),
    ("s", b"PONG\r\n"),
]


def corpus() -> list[Rec]:
    """The synthetic capture: four sessions on four server ports."""
    return (
        tcp_session(HTTP_TURNS, T0, sport=40001, dport=8081)
        + tcp_session(SMTP_TURNS, T0 + 1, sport=40002, dport=8025)
        + tcp_session(FRAME_TURNS, T0 + 2, sport=40003, dport=8443)
        + udp_session(UDP_TURNS, T0 + 3, sport=50001, dport=8053)
    )


def tree_digest(directory: Path) -> str:
    """Module tree digest of ``contracts/README.md``."""
    lines = []
    for path in sorted(p for p in directory.rglob("*") if p.is_file()):
        rel = path.relative_to(directory).as_posix()
        lines.append((rel, f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {rel}\n"))
    lines.sort(key=lambda item: item[0].encode())
    return "sha256:" + hashlib.sha256("".join(line for _, line in lines).encode()).hexdigest()


def build_plan(
    modules_dir: Path, client_pool: str, server_pool: str, rate: int, seconds: int
) -> dict[str, Any]:
    """An agent plan that runs every scenario of the generated module."""
    plan: dict[str, Any] = json.loads(EXAMPLE_PLAN.read_text())
    module_dir = modules_dir / MODULE
    scenarios = sorted((module_dir / "scenarios").glob("*.yaml"))
    sources = [(p.stem, p.read_text(encoding="utf-8")) for p in scenarios]
    plan["modules"] = [
        {"name": MODULE, "version": "0.1.0", "digest": tree_digest(module_dir)},
    ]
    plan["scenarios"] = [
        {
            "ref": f"{MODULE}/{name}",
            "module": MODULE,
            "source": text,
            "digest": "sha256:" + hashlib.sha256(text.encode()).hexdigest(),
        }
        for name, text in sources
    ]
    plan["mix"] = [
        {
            "id": name,
            "scenario": f"{MODULE}/{name}",
            "weight": 1 / len(sources),
            "clientPool": "clients",
            "serverPool": "servers",
        }
        for name, _ in sources
    ]
    plan["hostPools"][0]["ipv4Prefix"] = client_pool
    plan["hostPools"][1]["ipv4Prefix"] = server_pool
    for worker in plan["workers"]:
        worker["rates"] = [{"mixEntry": name, "sessionsPerSecond": rate} for name, _ in sources]
    plan["load"]["phases"] = [
        {"name": "ramp_up", "duration": "1s", "shape": "PHASE_SHAPE_LINEAR", "endFraction": 1},
        {
            "name": "steady",
            "duration": f"{seconds}s",
            "shape": "PHASE_SHAPE_HOLD",
            "startFraction": 1,
            "endFraction": 1,
        },
    ]
    plan["warmup"] = "1s"
    return plan


def compare(original: dict[str, Any], replay: dict[str, Any], tolerance: float = 0.01) -> list[str]:
    """Problems found when each replay session is held against the captured one."""
    problems: list[str] = []
    for want in original["sessions"]:
        port, proto = want["server"]["port"], want["protocol"]
        got = [
            s for s in replay["sessions"] if s["protocol"] == proto and s["server"]["port"] == port
        ]
        if not got:
            problems.append(f"port {port}: the replay has no {proto} session")
            continue
        key = "stream_bytes" if proto == "tcp" else "payload_bytes"
        for direction in ("client_to_server", "server_to_client"):
            target = want[direction][key]
            for s in got:
                size = s[direction][key]
                if abs(size - target) > tolerance * target:
                    problems.append(
                        f"port {port} {direction}: replay {size} bytes, capture {target} bytes"
                    )
    return problems


def run(cmd: list[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, text=True, check=False, **kwargs)


def replay_loopback(agent: Path, plan_path: Path, modules: Path, out: Path) -> Path:
    """Run both sides in one process on loopback pools and record ``lo``."""
    pcap = out / "replay.pcap"
    dump = subprocess.Popen(
        [
            "tcpdump",
            "-i",
            "lo",
            "-U",
            "-w",
            str(pcap),
            "-s",
            "0",
            "net 127.0.0.0/8 and not port 22",
        ],
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        time.sleep(1.0)
        proc = run(
            [str(agent), "run", "--plan", str(plan_path), "--modules", str(modules)],
            capture_output=True,
        )
        (out / "agent.log").write_text(proc.stderr)
        if proc.returncode != 0:
            raise SystemExit(f"the agent exited with {proc.returncode}\n{proc.stderr[-2000:]}")
    finally:
        time.sleep(0.5)
        dump.send_signal(signal.SIGINT)
        dump.wait(timeout=10)
    return pcap


def replay_netns(agent: Path, plan_path: Path, modules: Path, out: Path) -> Path:
    """Run the server side and the client side in the QA-01 namespaces and record the
    router's client arm."""
    env = {**os.environ, "FPS_NETNS": "cap02", "FPS_STATE_DIR": str(out / "state")}

    def harness(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        done = run([str(HARNESS), *args], env=env, capture_output=True)
        if check and done.returncode != 0:
            raise SystemExit(f"netns.sh {' '.join(args)} failed:\n{done.stderr}")
        return done

    plan = json.loads(plan_path.read_text())
    pools = {h["role"]: h["ipv4Prefix"] for h in plan["hostPools"]}
    pcap = out / "replay.pcapng"
    harness("up")
    try:
        info = dict(
            line.split("=", 1) for line in harness("info").stdout.splitlines() if "=" in line
        )
        router = ["ip", "netns", "exec", "cap02-router"]
        for cmd in (
            [*router, "ip", "route", "add", pools["HOST_ROLE_CLIENT"], "via", info["CLIENT_ADDR"]],
            [*router, "ip", "route", "add", pools["HOST_ROLE_SERVER"], "via", info["SERVER_ADDR"]],
        ):
            if run(cmd, capture_output=True).returncode != 0:
                raise SystemExit(f"{' '.join(cmd)} failed")
        harness(
            "exec", "client", "ip", "route", "add", "local", pools["HOST_ROLE_CLIENT"], "dev", "lo"
        )
        harness(
            "exec", "server", "ip", "route", "add", "local", pools["HOST_ROLE_SERVER"], "dev", "lo"
        )
        harness("capture", "start", str(pcap), "client")
        base = [str(agent), "run", "--plan", str(plan_path), "--modules", str(modules)]
        server = subprocess.Popen(
            [str(HARNESS), "exec", "server", *base, "--side", "server", "--linger", "5"],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=(out / "server.log").open("w"),
        )
        for _ in range(200):  # the server side is ready when it listens
            if harness("exec", "server", "ss", "-Hltn", check=False).stdout.strip():
                break
            if server.poll() is not None:
                raise SystemExit(f"the server side ended early\n{(out / 'server.log').read_text()}")
            time.sleep(0.1)
        client = run(
            [str(HARNESS), "exec", "client", *base, "--side", "client"],
            env=env,
            capture_output=True,
        )
        (out / "client.log").write_text(client.stderr)
        server_rc = server.wait(timeout=120)
        harness("capture", "stop")
        if client.returncode != 0 or server_rc != 0:
            raise SystemExit(
                f"agent exit codes: client {client.returncode}, server {server_rc}\n"
                f"{client.stderr[-2000:]}"
            )
    finally:
        harness("down", check=False)
    return pcap


def replay(agent: Path, mode: str, work: Path, rate: int = 20, seconds: int = 3) -> list[str]:
    """Generate, replay, and compare. Returns the problems, empty when the replay matches."""
    work.mkdir(parents=True, exist_ok=True)
    pcap = work / "original.pcap"
    pcap.write_bytes(write_pcap(sorted(corpus(), key=lambda r: r.t)))
    original = extract(pcap, payloads=True)
    sessions = work / "sessions.json"
    sessions.write_text(json.dumps(original))
    modules = work / "modules"
    code = main(
        ["scenario", "--sessions", str(sessions), "--all", "--out-dir", str(modules / MODULE),
         "--synthetic"]
    )  # fmt: skip
    if code != 0:
        return ["scenario generation failed"]
    if mode == "loopback":
        pools = ("127.71.0.0/24", "127.72.0.0/24")
    else:
        pools = ("16.0.0.0/24", "48.0.0.0/24")
    plan = build_plan(modules, pools[0], pools[1], rate, seconds)
    plan_path = work / "plan.json"
    plan_path.write_text(json.dumps(plan, indent=2))
    captured = (replay_loopback if mode == "loopback" else replay_netns)(
        agent, plan_path, modules, work
    )
    replayed = extract(captured, payloads=False)
    (work / "replay-sessions.json").write_text(json.dumps(replayed))
    problems = compare(original, replayed)
    count = len(replayed["sessions"])
    print(f"replayed {count} sessions in mode {mode}; {len(problems)} problems")
    if count < len(original["sessions"]):
        problems.append(f"only {count} replay sessions")
    return problems


def cli(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("netns", "loopback"), required=True)
    parser.add_argument("--agent", type=Path, required=True, help="path of the fps-agent binary")
    parser.add_argument(
        "--work", type=Path, help="directory for the files (default: a temporary one)"
    )
    args = parser.parse_args(argv)
    if os.geteuid() != 0:
        print("the replay needs root for tcpdump and namespaces", file=sys.stderr)
        return 2
    if shutil.which("tcpdump") is None:
        print("the replay needs tcpdump", file=sys.stderr)
        return 2
    work = args.work or Path(tempfile.mkdtemp(prefix="cap02-replay-"))
    problems = replay(args.agent.resolve(), args.mode, work)
    for p in problems:
        print(f"replay: {p}", file=sys.stderr)
    print(f"files in {work}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(cli())
