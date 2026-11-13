# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the transport behavior check: profiles, handshakes, strict mode, and the report.

The option bytes below are written out, not derived from the profile files, so the tests hold
the check to the public SYN signatures of Windows 11, Linux 6, and macOS 14 and not to its own
reading of them. The committed module fixtures give real captures of the kernel backend.
"""

from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest
import yaml

from realism import transport
from realism.capture import Frame
from realism.classifier import StubClassifier
from realism.cli import EXIT_ERROR, EXIT_FAIL, EXIT_PASS, main
from realism.labels import Labels, LabelsError, parse
from realism.report import LABELS_SCHEMA_PATH, SCHEMA_PATH, score
from realism.tests import pcapgen
from realism.tests.conftest import NDPI_42_HEADER

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "modules"
CLIENT = "10.201.0.2"
SERVER = "10.202.0.2"
TS = bytes.fromhex("0000000100000000")  # timestamp value and echo, which the check ignores

# The SYN of each system as the Transport page lists it, in bytes.
WIN_SYN = bytes.fromhex("020405b40103030801010402")  # mss nop ws8 nop nop sok
LINUX_SYN = bytes.fromhex("020405b40402080a") + TS + bytes.fromhex("01030307")
MAC_SYN = (
    bytes.fromhex("020405b4010303060101080a") + TS + bytes.fromhex("04020000")
)  # mss nop ws6 nop nop ts sok eol, padded to a word
WIN_SYN_ACK = bytes.fromhex("020405b40101040201030308")  # mss nop nop sok nop ws8
LINUX_SYN_ACK = LINUX_SYN
MAC_SYN_ACK = bytes.fromhex("020405b40402080a") + TS + bytes.fromhex("01030306")

# Profile, SYN, SYN-ACK, and the TTL and window that each shows at the capture point.
STACKS = {
    "windows11": (
        pcapgen.Handshake(ttl=128, window=64240, options=WIN_SYN),
        pcapgen.Handshake(ttl=127, window=64240, options=WIN_SYN_ACK),
    ),
    "linux6": (
        pcapgen.Handshake(ttl=64, window=64240, options=LINUX_SYN),
        pcapgen.Handshake(ttl=63, window=65160, options=LINUX_SYN_ACK),
    ),
    "macos14": (
        pcapgen.Handshake(ttl=64, window=65535, options=MAC_SYN),
        pcapgen.Handshake(ttl=63, window=65160, options=MAC_SYN_ACK),
    ),
}


def _labels(section: dict[str, Any] | None = None, **extra: Any) -> Labels:
    doc: dict[str, Any] = {"labels": "fps-realism-labels", "version": 1}
    doc["transport"] = {"os_profile": "linux6", "server_hops": 1} if section is None else section
    return parse(doc | extra)


def _capture(
    syn: pcapgen.Handshake,
    syn_ack: pcapgen.Handshake,
    count: int = 5,
    client: str = CLIENT,
    server: str = SERVER,
) -> pcapgen.Capture:
    cap = pcapgen.Capture()
    for i in range(count):
        cap.tcp_handshake((client, 40000 + i), (server, 80), syn, syn_ack)
    return cap


def _run(tmp_path: Path, cap: pcapgen.Capture, labels: Labels, **options: Any) -> Any:
    pcap = cap.write_pcap(tmp_path / "t.pcap")
    return transport.check(pcap, labels, **options)


def _profile_labels(name: str) -> Labels:
    return _labels({"os_profile": name, "server_hops": 1})


# --- acceptance: each OS profile fixture passes ---------------------------------------------


@pytest.mark.parametrize("name", ["windows11", "linux6", "macos14"])
def test_the_fixture_of_each_os_profile_passes(tmp_path: Path, name: str) -> None:
    syn, syn_ack = STACKS[name]
    result = _run(tmp_path, _capture(syn, syn_ack), _profile_labels(name))
    assert result.passed, result.failures
    assert result.failures == []
    assert (result.syns_total, result.syn_acks_total, result.unprofiled_handshakes) == (5, 5, 0)
    entry = result.to_dict()
    assert entry["check"] == "transport_behavior"
    assert [(h["host"], h["syns"], h["syn_acks"]) for h in entry["hosts"]] == [
        (CLIENT, 5, 0),
        (SERVER, 0, 5),
    ]


def test_the_profile_files_hold_the_public_signatures() -> None:
    profiles = {n: transport.load_profile(n, transport.PROFILES_DIR) for n in STACKS}
    win, linux, mac = profiles["windows11"], profiles["linux6"], profiles["macos14"]
    assert (win.ttl, win.mss, win.window_scale, win.syn_window) == (128, 1460, 8, 64240)
    assert (linux.ttl, linux.mss, linux.window_scale, linux.syn_window) == (64, 1460, 7, 64240)
    assert (mac.ttl, mac.mss, mac.window_scale, mac.syn_window) == (64, 1460, 6, 65535)
    assert win.options == ("mss", "nop", "ws", "nop", "nop", "sok")
    assert linux.options == ("mss", "sok", "ts", "nop", "ws")
    assert mac.options == ("mss", "nop", "ws", "nop", "nop", "ts", "sok", "eol")
    assert (win.sack, win.timestamps) == (True, False)
    assert (linux.sack, linux.timestamps) == (True, True)


def test_the_option_bytes_read_back_as_the_signatures() -> None:
    assert transport.parse_options(WIN_SYN) == (
        ("mss", "nop", "ws", "nop", "nop", "sok"),
        1460,
        8,
    )
    assert transport.parse_options(LINUX_SYN) == (("mss", "sok", "ts", "nop", "ws"), 1460, 7)
    assert transport.parse_options(MAC_SYN) == (
        ("mss", "nop", "ws", "nop", "nop", "ts", "sok", "eol"),
        1460,
        6,
    )


# --- acceptance: a fixture with a wrong TTL fails --------------------------------------------


def test_a_syn_with_the_ttl_of_linux_fails_the_windows_profile(tmp_path: Path) -> None:
    syn, syn_ack = STACKS["windows11"]
    cap = _capture(pcapgen.Handshake(64, syn.window, syn.options), syn_ack)
    result = _run(tmp_path, cap, _profile_labels("windows11"))
    assert not result.passed
    assert result.failures == [
        f"host {CLIENT} (profile windows11): 5 of 5 SYN packets differ in the IP TTL: "
        "the profile expects 128, the capture shows 64 in 5 of them"
    ]
    (dev,) = result.deviations()
    assert dev | {"observed": dict(dev["observed"])} == {
        "host": CLIENT,
        "profile": "windows11",
        "role": "syn",
        "field": "ttl",
        "expected": "128",
        "observed": {"64": 5},
        "handshakes": 5,
        "mismatched": 5,
        "enforced": True,
    }


@pytest.mark.parametrize(("ttl", "shown"), [(65, "65"), (63, "63"), (128, "128"), (255, "255")])
def test_the_ttl_must_equal_the_profile_less_the_hops(tmp_path: Path, ttl: int, shown: str) -> None:
    _syn, syn_ack = STACKS["linux6"]
    result = _run(tmp_path, _capture(pcapgen.Handshake(ttl, 64240, LINUX_SYN), syn_ack), _labels())
    assert not result.passed
    assert result.failures == [
        f"host {CLIENT} (profile linux6): 5 of 5 SYN packets differ in the IP TTL: "
        f"the profile expects 64, the capture shows {shown} in 5 of them"
    ]


def test_a_wrong_ttl_on_the_syn_ack_fails_with_the_hops_applied(tmp_path: Path) -> None:
    syn, _ = STACKS["linux6"]
    cap = _capture(syn, pcapgen.Handshake(64, 65160, LINUX_SYN_ACK))
    result = _run(tmp_path, cap, _labels())
    assert result.failures == [
        f"host {SERVER} (profile linux6): 5 of 5 SYN-ACK packets differ in the IP TTL: "
        "the profile expects 63, the capture shows 64 in 5 of them"
    ]


def test_the_hops_move_the_expected_ttl(tmp_path: Path) -> None:
    syn, syn_ack = STACKS["windows11"]
    cap = _capture(pcapgen.Handshake(125, syn.window, syn.options), syn_ack)
    far = _run(tmp_path, cap, _labels({"os_profile": "windows11", "client_hops": 3}))
    assert far.failures == [
        f"host {SERVER} (profile windows11): 5 of 5 SYN-ACK packets differ in the IP TTL: "
        "the profile expects 128, the capture shows 127 in 5 of them"
    ]
    near = _run(
        tmp_path, cap, _labels({"os_profile": "windows11", "client_hops": 3, "server_hops": 1})
    )
    assert near.passed, near.failures


# --- the other fields ------------------------------------------------------------------------


def _fails_with(tmp_path: Path, name: str, syn: pcapgen.Handshake, text: str) -> None:
    _, syn_ack = STACKS[name]
    result = _run(tmp_path, _capture(syn, syn_ack), _profile_labels(name))
    assert not result.passed
    assert [f for f in result.failures if text in f], result.failures


def test_a_wrong_mss_fails(tmp_path: Path) -> None:
    bad = LINUX_SYN.replace(bytes.fromhex("05b4"), bytes.fromhex("0218"), 1)
    _fails_with(
        tmp_path,
        "linux6",
        pcapgen.Handshake(64, 64240, bad),
        "5 of 5 SYN packets differ in the MSS option: the profile expects 1460, "
        "the capture shows 536 in 5 of them",
    )


def test_a_wrong_window_scale_fails(tmp_path: Path) -> None:
    bad = LINUX_SYN[:-1] + b"\x0a"
    _fails_with(
        tmp_path,
        "linux6",
        pcapgen.Handshake(64, 64240, bad),
        "differ in the window scale option: the profile expects 7, the capture shows 10 in 5",
    )


def test_timestamps_on_a_windows_syn_fail(tmp_path: Path) -> None:
    _fails_with(
        tmp_path,
        "windows11",
        pcapgen.Handshake(128, 64240, LINUX_SYN),
        "differ in the timestamp option: the profile expects no, the capture shows yes in 5",
    )


def test_missing_sack_fails(tmp_path: Path) -> None:
    no_sack = bytes.fromhex("020405b40103030801010101")
    _fails_with(
        tmp_path,
        "windows11",
        pcapgen.Handshake(128, 64240, no_sack),
        "differ in the SACK permitted option: the profile expects yes, the capture shows no in 5",
    )


def test_a_syn_without_the_do_not_fragment_flag_fails(tmp_path: Path) -> None:
    _fails_with(
        tmp_path,
        "linux6",
        pcapgen.Handshake(64, 64240, LINUX_SYN, df=False),
        "differ in the do not fragment flag: the profile expects yes, the capture shows no in 5",
    )


def test_a_syn_without_window_scaling_fails_a_profile_that_has_it(tmp_path: Path) -> None:
    no_ws = bytes.fromhex("020405b40402080a") + TS + bytes.fromhex("01010101")
    _fails_with(
        tmp_path,
        "linux6",
        pcapgen.Handshake(64, 64240, no_ws),
        "differ in the window scale option: the profile expects 7, the capture shows none in 5",
    )


def test_several_wrong_fields_give_one_failure_each(tmp_path: Path) -> None:
    _syn, syn_ack = STACKS["windows11"]
    result = _run(
        tmp_path,
        _capture(pcapgen.Handshake(64, 64240, LINUX_SYN), syn_ack),
        _profile_labels("windows11"),
    )
    fields = sorted(d["field"] for d in result.deviations() if d["role"] == "syn")
    assert fields == ["options", "timestamps", "ttl", "window_scale"]
    assert len(result.failures) == 3  # the option order is not enforced
    entry = result.to_dict()
    assert entry["failures_total"] == 3
    assert entry["passed"] is False


# --- the option order and the SYN window need strict ---------------------------------------


def _kernel_macos() -> pcapgen.Capture:
    # What the kernel backend sends for macos14: Linux order and a window of whole segments.
    linux_order = bytes.fromhex("020405b40402080a") + TS + bytes.fromhex("01030306")
    return _capture(
        pcapgen.Handshake(64, 64240, linux_order), pcapgen.Handshake(63, 65160, MAC_SYN_ACK)
    )


def test_the_option_order_and_the_syn_window_are_reported_but_not_enforced(tmp_path: Path) -> None:
    result = _run(tmp_path, _kernel_macos(), _profile_labels("macos14"))
    assert result.passed, result.failures
    assert [
        (d["field"], d["enforced"], d["expected"], d["observed"]) for d in result.deviations()
    ] == [
        (
            "options",
            False,
            "mss,nop,ws,nop,nop,ts,sok,eol",
            {"mss,sok,ts,nop,ws": 5},
        ),
        ("syn_window", False, "65535", {"64240": 5}),
    ]


def test_strict_mode_enforces_the_option_order_and_the_syn_window(tmp_path: Path) -> None:
    labels = _labels({"os_profile": "macos14", "server_hops": 1, "strict": True})
    result = _run(tmp_path, _kernel_macos(), labels)
    assert not result.passed
    assert result.failures == [
        f"host {CLIENT} (profile macos14): 5 of 5 SYN packets differ in the TCP option list: "
        "the profile expects mss,nop,ws,nop,nop,ts,sok,eol, "
        "the capture shows mss,sok,ts,nop,ws in 5 of them",
        f"host {CLIENT} (profile macos14): 5 of 5 SYN packets differ in the SYN window: "
        "the profile expects 65535, the capture shows 64240 in 5 of them",
    ]
    syn, syn_ack = STACKS["macos14"]
    assert _run(tmp_path, _capture(syn, syn_ack), labels).passed
    override = _run(tmp_path, _kernel_macos(), _profile_labels("macos14"), strict=True)
    assert not override.passed


# --- SYN-ACK negotiation ---------------------------------------------------------------------


def test_a_syn_ack_may_not_carry_an_option_that_the_syn_lacked(tmp_path: Path) -> None:
    # A Windows client offers no timestamps, so a Windows server must not answer with them.
    syn, _ = STACKS["windows11"]
    answer = pcapgen.Handshake(127, 64240, LINUX_SYN_ACK)
    result = _run(tmp_path, _capture(syn, answer), _profile_labels("windows11"))
    texts = sorted(result.failures)
    assert texts == [
        f"host {SERVER} (profile windows11): 5 of 5 SYN-ACK packets differ in the timestamp "
        "option: the profile expects no, the capture shows yes in 5 of them",
        f"host {SERVER} (profile windows11): 5 of 5 SYN-ACK packets differ in the window scale "
        "option: the profile expects 8, the capture shows 7 in 5 of them",
    ]


def test_a_syn_ack_answers_only_the_options_that_the_syn_offered(tmp_path: Path) -> None:
    # A client that offers neither timestamps nor window scaling gets a Linux answer without them.
    plain = bytes.fromhex("020405b404020101")
    cap = _capture(pcapgen.Handshake(64, 64240, plain), pcapgen.Handshake(63, 65160, plain))
    server_only = _labels({"hosts": {SERVER: "linux6"}, "server_hops": 1})
    result = _run(tmp_path, cap, server_only)
    assert result.passed, result.failures
    assert (result.syns_total, result.syn_acks_total, result.unprofiled_handshakes) == (5, 5, 5)
    # The same answer to a SYN that offered them lacks what the profile would send back.
    syn, _ = STACKS["linux6"]
    full = _run(tmp_path, _capture(syn, pcapgen.Handshake(63, 65160, plain)), server_only)
    assert sorted(
        f.split(": ", 1)[1].split(" differ in ")[1].split(":")[0] for f in full.failures
    ) == [
        "the timestamp option",
        "the window scale option",
    ]


def test_a_syn_ack_without_its_syn_is_compared_in_ttl_and_mss_only(tmp_path: Path) -> None:
    _, syn_ack = STACKS["linux6"]
    cap = pcapgen.Capture()
    cap.add(
        pcapgen.tcp(
            SERVER,
            80,
            CLIENT,
            40000,
            pcapgen.SYN | pcapgen.ACK,
            5000,
            1001,
            ttl=63,
            window=65160,
            options=bytes.fromhex("020405b401010101"),
        )
    )
    result = _run(tmp_path, cap, _labels())
    assert result.passed, result.failures
    assert (result.syns_total, result.syn_acks_total) == (0, 1)
    wrong = pcapgen.Capture()
    wrong.add(
        pcapgen.tcp(
            SERVER,
            80,
            CLIENT,
            40000,
            pcapgen.SYN | pcapgen.ACK,
            5000,
            1001,
            ttl=63,
            window=65160,
            options=bytes.fromhex("0204021801010101"),
        )
    )
    assert any("MSS option" in f for f in _run(tmp_path, wrong, _labels()).failures)
    del syn_ack


# --- hosts, retransmissions, and what the check leaves out -----------------------------------


def test_each_host_is_compared_with_its_own_profile(tmp_path: Path) -> None:
    win_syn, win_ack = STACKS["windows11"]
    lin_syn, lin_ack = STACKS["linux6"]
    cap = pcapgen.Capture()
    for i in range(3):
        cap.tcp_handshake(("10.0.0.10", 41000 + i), ("10.0.1.80", 80), win_syn, win_ack)
    for i in range(4):
        cap.tcp_handshake(("10.0.0.11", 42000 + i), ("10.0.1.80", 80), lin_syn, lin_ack)
    section = {
        "os_profile": "linux6",
        "hosts": {"10.0.0.10": "windows11"},
        "server_hops": 1,
    }
    result = _run(tmp_path, cap, _labels(section))
    # The default profile also judges the server, which answers the Windows client in Windows
    # form, so the server host shows deviations while both clients match their profiles.
    assert [
        (h["host"], h["profile"], h["syns"], h["syn_acks"]) for h in result.to_dict()["hosts"]
    ] == [
        ("10.0.0.10", "windows11", 3, 0),
        ("10.0.0.11", "linux6", 4, 0),
        ("10.0.1.80", "linux6", 0, 7),
    ]
    assert all("10.0.0.10" not in f and "10.0.0.11" not in f for f in result.failures)
    assert [d["field"] for d in result.deviations() if d["host"] == "10.0.1.80"] != []


def test_a_host_with_the_wrong_stack_is_named(tmp_path: Path) -> None:
    win_syn, win_ack = STACKS["windows11"]
    _lin_syn, _lin_ack = STACKS["linux6"]
    cap = pcapgen.Capture()
    for i in range(3):
        cap.tcp_handshake(("10.0.0.10", 41000 + i), ("10.0.1.80", 80), win_syn, win_ack)
    for i in range(2):
        cap.tcp_handshake(("10.0.0.11", 42000 + i), ("10.0.1.81", 80), win_syn, win_ack)
    section = {"hosts": {"10.0.0.10": "windows11", "10.0.0.11": "linux6"}, "server_hops": 1}
    result = _run(tmp_path, cap, _labels(section))
    assert result.unprofiled_handshakes == 5  # the two servers have no profile
    assert [f for f in result.failures if "10.0.0.10" in f] == []
    assert (
        "host 10.0.0.11 (profile linux6): 2 of 2 SYN packets differ in the IP TTL: "
        in (result.failures[0])
    )
    assert "the profile expects 64, the capture shows 128 in 2 of them" in result.failures[0]


def test_a_retransmitted_syn_counts_once(tmp_path: Path) -> None:
    _syn, _syn_ack = STACKS["linux6"]
    cap = pcapgen.Capture()
    cap.add(
        pcapgen.tcp(
            CLIENT, 40000, SERVER, 80, pcapgen.SYN, 1000, 0, ttl=64, window=64240, options=LINUX_SYN
        )
    )
    cap.add(
        pcapgen.tcp(
            CLIENT, 40000, SERVER, 80, pcapgen.SYN, 1000, 0, ttl=64, window=64240, options=LINUX_SYN
        )
    )
    cap.add(
        pcapgen.tcp(
            SERVER,
            80,
            CLIENT,
            40000,
            pcapgen.SYN | pcapgen.ACK,
            5000,
            1001,
            ttl=63,
            window=65160,
            options=LINUX_SYN_ACK,
        )
    )
    result = _run(tmp_path, cap, _labels())
    assert result.passed, result.failures
    assert (result.syns_total, result.syn_acks_total) == (1, 1)


def test_a_capture_without_a_profiled_handshake_fails(tmp_path: Path) -> None:
    syn, syn_ack = STACKS["linux6"]
    none = _run(tmp_path, pcapgen.Capture(), _labels())
    assert none.failures == [
        "the capture holds no TCP handshake, so the transport behavior is not judged"
    ]
    other = _run(tmp_path, _capture(syn, syn_ack), _labels({"hosts": {"10.9.9.9": "linux6"}}))
    assert other.failures == [
        "no TCP handshake of the capture comes from a host with an OS profile, "
        "so the transport behavior is not judged"
    ]
    assert other.unprofiled_handshakes == 10


def test_a_malformed_option_list_fails(tmp_path: Path) -> None:
    broken = bytes.fromhex("020405b404000101")  # SACK permitted with length 0
    _syn, syn_ack = STACKS["linux6"]
    result = _run(tmp_path, _capture(pcapgen.Handshake(64, 64240, broken), syn_ack), _labels())
    assert result.failures == [
        f"host {CLIENT} (profile linux6): 5 of 5 SYN packets differ in the TCP option list: "
        "the profile expects well formed, the capture shows malformed in 5 of them"
    ]
    assert transport.parse_options(bytes.fromhex("0204"))[0] == ("mss", "malformed")


def test_an_ipv6_handshake_uses_the_hop_limit_and_has_no_do_not_fragment_flag(
    tmp_path: Path,
) -> None:
    syn, syn_ack = STACKS["linux6"]
    cap = _capture(syn, syn_ack, client="fd00::10", server="fd00::80")
    result = _run(tmp_path, cap, _labels())
    assert result.passed, result.failures
    assert not any(d["field"] == "df" for d in result.deviations())
    bad = _capture(
        pcapgen.Handshake(128, 64240, LINUX_SYN), syn_ack, client="fd00::10", server="fd00::80"
    )
    assert (
        _run(tmp_path, bad, _labels())
        .failures[0]
        .endswith("the profile expects 64, the capture shows 128 in 5 of them")
    )


def test_the_expect_section_replaces_the_profile_values(tmp_path: Path) -> None:
    _syn, _syn_ack = STACKS["linux6"]
    cap = _capture(
        pcapgen.Handshake(60, 64240, LINUX_SYN), pcapgen.Handshake(59, 65160, LINUX_SYN_ACK)
    )
    assert not _run(tmp_path, cap, _labels()).passed
    section = {"os_profile": "linux6", "server_hops": 1, "expect": {"ttl": 60}}
    assert _run(tmp_path, cap, _labels(section)).passed
    entry = _run(tmp_path, cap, _labels(section)).to_dict()
    assert entry["expect"] == {"ttl": 60}
    assert entry["profiles"][0]["ttl"] == 60


def test_a_non_ip_frame_is_not_a_handshake() -> None:
    assert transport.decode_handshake(Frame(0.0, 1, b"\x00" * 10)) is None
    assert transport.decode_handshake(Frame(0.0, 1, b"\x00" * 60)) is None


# --- committed fixtures of the kernel backend -----------------------------------------------

# Module, connections, and the MSS of the fixture. The harness namespaces give window scale 10.
FIXTURE_CASES = [
    ("dns", 20, 1460),
    ("http1", 200, 1460),
    ("http2", 80, 1460),
    ("imap", 48, 1460),
    ("smtp", 56, 1460),
    ("ssh", 16, 1460),
    ("ftp", 120, 65495),
    ("pop3", 40, 65495),
]


def _fixture(module: str) -> Path:
    path = FIXTURES / module / "fixtures" / f"{module}.pcapng"
    if not path.is_file():
        pytest.skip(f"{path} is not in this checkout")
    return path


@pytest.mark.parametrize(("module", "connections", "mss"), FIXTURE_CASES)
def test_the_kernel_backend_fixtures_match_the_linux_profile_in_every_field_but_one(
    module: str, connections: int, mss: int
) -> None:
    hops = 0 if mss > 1460 else 1  # the loopback fixtures have no router
    labels = _labels({"os_profile": "linux6", "server_hops": hops, "expect": {"mss": mss}})
    result = transport.check(_fixture(module), labels)
    assert (result.syns_total, result.syn_acks_total) == (connections, connections)
    assert {(d["role"], d["field"], tuple(d["observed"].items())) for d in result.deviations()} == {
        ("syn", "window_scale", (("10", connections),)),
        ("syn-ack", "window_scale", (("10", connections),)),
    } | ({("syn", "syn_window", ((str(65495), connections),))} if mss > 1460 else set())
    assert not result.passed
    fixed = _labels(
        {"os_profile": "linux6", "server_hops": hops, "expect": {"mss": mss, "window_scale": 10}}
    )
    assert transport.check(_fixture(module), fixed).passed


def test_a_real_linux_capture_fails_the_windows_profile_in_ttl_and_timestamps() -> None:
    labels = _labels({"os_profile": "windows11", "server_hops": 1})
    result = transport.check(_fixture("dns"), labels)
    assert result.failures == [
        "host 10.201.0.2 (profile windows11): 20 of 20 SYN packets differ in the IP TTL: "
        "the profile expects 128, the capture shows 64 in 20 of them",
        "host 10.201.0.2 (profile windows11): 20 of 20 SYN packets differ in the window scale "
        "option: the profile expects 8, the capture shows 10 in 20 of them",
        "host 10.201.0.2 (profile windows11): 20 of 20 SYN packets differ in the timestamp "
        "option: the profile expects no, the capture shows yes in 20 of them",
        "host 10.202.0.2 (profile windows11): 20 of 20 SYN-ACK packets differ in the IP TTL: "
        "the profile expects 127, the capture shows 63 in 20 of them",
        "host 10.202.0.2 (profile windows11): 20 of 20 SYN-ACK packets differ in the window scale "
        "option: the profile expects 8, the capture shows 10 in 20 of them",
        "host 10.202.0.2 (profile windows11): 20 of 20 SYN-ACK packets differ in the timestamp "
        "option: the profile expects no, the capture shows yes in 20 of them",
    ]


def test_the_server_ttl_of_a_fixture_needs_the_hop_of_the_router() -> None:
    result = transport.check(_fixture("http1"), _labels({"os_profile": "linux6"}))
    assert (
        f"host {SERVER} (profile linux6): 200 of 200 SYN-ACK packets differ in the IP TTL: "
        "the profile expects 64, the capture shows 63 in 200 of them"
    ) in result.failures


# --- profile files ---------------------------------------------------------------------------


def _profile(tmp_path: Path, text: str, name: str = "demo") -> Path:
    (tmp_path / f"{name}.yaml").write_text(text)
    return tmp_path


GOOD_PROFILE = """\
name: demo
socket: {ttl: 100, mss: 1400}
reference: {syn_window: 8192, window_scale: 2, options: [mss, nop, ws], dont_fragment: false}
"""


def test_a_profile_from_another_directory_is_used(tmp_path: Path) -> None:
    directory = _profile(tmp_path, GOOD_PROFILE)
    mss_ws = bytes.fromhex("0204057801030302")
    syn = pcapgen.Handshake(100, 8192, mss_ws, df=False)
    cap = _capture(syn, pcapgen.Handshake(100, 8192, mss_ws, df=False))
    result = _run(tmp_path, cap, _labels({"os_profile": "demo"}), profiles_dir=directory)
    assert result.passed, result.failures
    assert result.to_dict()["profiles"] == [
        {
            "name": "demo",
            "file": "demo.yaml",
            "ttl": 100,
            "mss": 1400,
            "window_scale": 2,
            "syn_window": 8192,
            "options": ["mss", "nop", "ws"],
            "dont_fragment": False,
        }
    ]


def test_the_labels_name_the_profile_directory_relative_to_the_labels_file(tmp_path: Path) -> None:
    _profile(tmp_path, GOOD_PROFILE)
    doc = {
        "labels": "fps-realism-labels",
        "version": 1,
        "transport": {"os_profile": "demo", "profiles_dir": "."},
    }
    path = tmp_path / "labels.json"
    path.write_text(json.dumps(doc))
    from realism.labels import load

    labels = load(path)
    syn, syn_ack = STACKS["linux6"]
    result = _run(tmp_path, _capture(syn, syn_ack), labels)
    assert result.profiles[0].name == "demo"
    assert not result.passed


@pytest.mark.parametrize(
    ("text", "message"),
    [
        ("name: demo\nsocket: {ttl: 100, mss: 1400}\n", "the section reference is missing"),
        (GOOD_PROFILE.replace("ttl: 100", "ttl: 0"), "socket.ttl and socket.mss must be integers"),
        (GOOD_PROFILE.replace("window_scale: 2", "window_scale: 15"), "reference needs"),
        (GOOD_PROFILE.replace("[mss, nop, ws]", "[mss, bogus]"), "reference.options must list"),
        (GOOD_PROFILE.replace("name: demo", "name: other"), "its name is 'other', not 'demo'"),
        ("- not a mapping\n", "must be a mapping with a name"),
    ],
)
def test_a_bad_profile_is_refused(tmp_path: Path, text: str, message: str) -> None:
    directory = _profile(tmp_path, text)
    with pytest.raises(transport.TransportError, match=message):
        transport.load_profile("demo", directory)


def test_a_missing_profile_names_the_directory(tmp_path: Path) -> None:
    with pytest.raises(transport.TransportError, match="profile beos is not in"):
        transport.load_profile("beos", tmp_path)


def test_the_check_needs_a_profile(tmp_path: Path) -> None:
    labels = parse({"labels": "fps-realism-labels", "version": 1})
    with pytest.raises(LabelsError, match="name an os_profile or hosts"):
        _run(tmp_path, pcapgen.Capture(), labels, strict=True)


# --- labels section --------------------------------------------------------------------------

BAD_SECTIONS: list[Any] = [
    {},
    {"os_profile": "Linux 6"},
    {"os_profile": "linux6", "extra": 1},
    {"hosts": {}},
    {"hosts": {"not an address": "linux6"}},
    {"hosts": {"10.0.0.1": "BAD"}},
    {"os_profile": "linux6", "client_hops": -1},
    {"os_profile": "linux6", "server_hops": 255},
    {"os_profile": "linux6", "server_hops": True},
    {"os_profile": "linux6", "strict": "yes"},
    {"os_profile": "linux6", "profiles_dir": ""},
    {"os_profile": "linux6", "expect": {"window": 3}},
    {"os_profile": "linux6", "expect": {"ttl": -1}},
    [],
]


def test_the_labels_section_is_validated_by_the_schema_and_the_parser() -> None:
    schema = jsonschema.Draft202012Validator(json.loads(LABELS_SCHEMA_PATH.read_text()))
    base = {"labels": "fps-realism-labels", "version": 1}
    good = {
        "os_profile": "linux6",
        "hosts": {"10.0.0.10": "windows11", "fd00::1": "macos14"},
        "profiles_dir": "profiles",
        "client_hops": 0,
        "server_hops": 1,
        "strict": True,
        "expect": {"ttl": 60, "mss": 1400, "window_scale": 10, "syn_window": 64240},
    }
    assert schema.is_valid(base | {"transport": good})
    parsed = parse(base | {"transport": good}).transport
    assert parsed is not None
    assert (parsed.os_profile, parsed.server_hops, parsed.strict) == ("linux6", 1, True)
    assert dict(parsed.hosts) == {"10.0.0.10": "windows11", "fd00::1": "macos14"}
    for section in BAD_SECTIONS:
        assert not schema.is_valid(base | {"transport": section}), section
        with pytest.raises(LabelsError):
            parse(base | {"transport": section})


# --- report and command line ----------------------------------------------------------------


def _validate(report: dict[str, Any]) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator(schema).validate(report)


def _labels_file(tmp_path: Path, section: dict[str, Any] | None) -> Path:
    doc: dict[str, Any] = {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [{"application": "http1", "match": {"transport": "tcp", "server_port": 80}}],
    }
    if section is not None:
        doc["transport"] = section
    path = tmp_path / "labels.json"
    path.write_text(json.dumps(doc))
    return path


STUB = StubClassifier.by_server_port({80: "HTTP"}, name="ndpi")


def test_the_report_has_the_transport_entry_and_validates(tmp_path: Path) -> None:
    syn, syn_ack = STACKS["windows11"]
    good = _capture(syn, syn_ack).write_pcap(tmp_path / "good.pcap")
    labels = _labels_file(tmp_path, {"os_profile": "windows11", "server_hops": 1})
    report = score(good, labels, STUB).to_dict()
    _validate(report)
    assert report["version"] == "0.20.0"
    assert [c["check"] for c in report["checks"]] == ["conformance", "transport_behavior"]
    entry = report["checks"][1]
    assert entry["passed"] is True
    assert (entry["syns_total"], entry["syn_acks_total"], entry["deviations"]) == (5, 5, [])
    bad = _capture(pcapgen.Handshake(64, syn.window, syn.options), syn_ack).write_pcap(
        tmp_path / "bad.pcap"
    )
    failed = score(bad, labels, STUB).to_dict()
    _validate(failed)
    assert failed["checks"][1]["passed"] is False
    assert failed["passed"] is False
    assert failed["checks"][1]["failures_total"] == 1


def test_the_report_of_a_strict_run_validates(tmp_path: Path) -> None:
    pcap = _kernel_macos().write_pcap(tmp_path / "k.pcap")
    labels = _labels_file(tmp_path, {"os_profile": "macos14", "server_hops": 1, "strict": True})
    report = score(pcap, labels, STUB).to_dict()
    _validate(report)
    assert report["checks"][1]["strict"] is True
    assert [d["field"] for d in report["checks"][1]["deviations"]] == ["options", "syn_window"]
    assert {d["enforced"] for d in report["checks"][1]["deviations"]} == {True}


def test_the_check_is_off_without_a_section_or_an_option(tmp_path: Path) -> None:
    syn, syn_ack = STACKS["linux6"]
    pcap = _capture(syn, syn_ack).write_pcap(tmp_path / "t.pcap")
    labels = _labels_file(tmp_path, None)
    assert [c["check"] for c in score(pcap, labels, STUB).to_dict()["checks"]] == ["conformance"]
    on = score(pcap, labels, STUB, os_profile="linux6", server_hops=1).to_dict()
    assert [c["check"] for c in on["checks"]] == ["conformance", "transport_behavior"]
    assert on["checks"][1]["passed"] is True
    with pytest.raises(LabelsError, match="name an os_profile or hosts"):
        score(pcap, labels, STUB, client_hops=1)


def test_the_command_line_adds_and_overrides_the_check(
    tmp_path: Path,
    fake_reader: Callable[..., Path],
    capsys: pytest.CaptureFixture[str],
) -> None:
    csv = NDPI_42_HEADER + "\n"
    for i in range(5):
        csv += f"{i},6,1,1,0,{CLIENT},{40000 + i},{SERVER},80,7,HTTP,,6,414\n"
    syn, syn_ack = STACKS["linux6"]
    pcap = _capture(syn, syn_ack).write_pcap(tmp_path / "t.pcap")
    base = ["score", "--pcap", str(pcap), "--parsers", "none"]
    base += ["--ndpi-reader", str(fake_reader(csv))]
    plain = [*base, "--labels", str(_labels_file(tmp_path, None))]
    assert main(plain) == EXIT_PASS
    capsys.readouterr()
    assert main([*plain, "--os-profile", "linux6", "--server-hops", "1"]) == EXIT_PASS
    report = json.loads(capsys.readouterr().out)
    assert report["checks"][1]["check"] == "transport_behavior"
    assert report["version"] == "0.20.0"
    assert main([*plain, "--os-profile", "windows11", "--server-hops", "1"]) == EXIT_FAIL
    out = json.loads(capsys.readouterr().out)
    assert out["checks"][1]["failures"][0].startswith(
        f"host {CLIENT} (profile windows11): 5 of 5 SYN packets differ in the IP TTL: "
        "the profile expects 128, the capture shows 64 in 5 of them"
    )
    assert main([*plain, "--os-profile", "linux6"]) == EXIT_FAIL  # the SYN-ACK is one hop down
    capsys.readouterr()
    strict = [*plain, "--server-hops", "1", "--transport-strict", "--os-profile"]
    assert main([*strict, "linux6"]) == EXIT_PASS  # the order and the window match
    assert json.loads(capsys.readouterr().out)["checks"][1]["strict"] is True
    assert main([*strict, "macos14"]) == EXIT_FAIL
    assert "differ in the TCP option list" in capsys.readouterr().out
    with_section = [*base, "--labels", str(_labels_file(tmp_path, {"os_profile": "macos14"}))]
    assert main([*with_section, "--os-profile", "linux6", "--server-hops", "1"]) == EXIT_PASS
    capsys.readouterr()


def test_the_command_line_refuses_a_missing_profile(
    tmp_path: Path,
    fake_reader: Callable[..., Path],
    capsys: pytest.CaptureFixture[str],
) -> None:
    syn, syn_ack = STACKS["linux6"]
    pcap = _capture(syn, syn_ack).write_pcap(tmp_path / "t.pcap")
    args = ["score", "--pcap", str(pcap), "--parsers", "none", "--labels"]
    args += [str(_labels_file(tmp_path, None)), "--ndpi-reader", str(fake_reader(NDPI_42_HEADER))]
    assert main([*args, "--os-profile", "beos"]) == EXIT_ERROR
    assert "profile beos is not in" in capsys.readouterr().err
    with pytest.raises(SystemExit):
        main([*args, "--os-profile", "linux6", "--client-hops", "300"])


def test_the_example_labels_pass_on_the_http1_fixture() -> None:
    path = ROOT / "realism" / "examples" / "transport.labels.yaml"
    doc = yaml.safe_load(path.read_text())
    jsonschema.validate(doc, json.loads(LABELS_SCHEMA_PATH.read_text()))
    labels = parse(doc)
    assert labels.transport is not None
    assert dict(labels.transport.expect) == {"window_scale": 10}
    result = transport.check(_fixture("http1"), labels)
    assert result.passed, result.failures
    assert (result.syns_total, result.syn_acks_total) == (200, 200)
