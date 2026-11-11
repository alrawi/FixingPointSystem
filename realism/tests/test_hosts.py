# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the host identity check: labels, host counting, address conflicts, and the report.

Every capture is built from code. The tests need no tool, because the check reads Ethernet
frames itself.
"""

from __future__ import annotations

import json
import struct
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from realism import hosts
from realism.capture import LINKTYPE_RAW
from realism.classifier import StubClassifier
from realism.cli import EXIT_FAIL, main
from realism.labels import Count, HostsLabels, LabelsError, parse
from realism.report import REPORT_VERSION, SCHEMA_PATH, score
from realism.tests import pcapgen

MAC_A = "02:00:00:00:0a:01"
MAC_B = "02:00:00:00:0b:02"
MAC_C = "02:00:00:00:0c:03"
GATEWAY = "02:00:00:00:00:fe"
BROADCAST = "ff:ff:ff:ff:ff:ff"


def _mac(text: str) -> bytes:
    return bytes.fromhex(text.replace(":", ""))


def _frame(
    src_mac: str,
    ip_packet: bytes,
    dst_mac: str = MAC_C,
    vlan: tuple[int, ...] = (),
) -> bytes:
    """Wrap an IP packet in an Ethernet header with the given MACs and VLAN tags."""
    ethertype = 0x0800 if ip_packet[0] >> 4 == 4 else 0x86DD
    tags = b""
    for position, tag in enumerate(vlan):
        outer = 0x8100 if position < len(vlan) - 1 else ethertype
        tags += struct.pack("!HH", tag, outer)
    first = 0x8100 if vlan else ethertype
    return _mac(dst_mac) + _mac(src_mac) + struct.pack("!H", first) + tags + ip_packet


def _arp(
    src_mac: str, sender_mac: str, sender_ip: str, target_ip: str, op: int = 1, dst: str = BROADCAST
) -> bytes:
    body = struct.pack("!HHBBH", 1, 0x0800, 6, 4, op) + _mac(sender_mac)
    body += bytes(int(p) for p in sender_ip.split("."))
    body += bytes(6) + bytes(int(p) for p in target_ip.split("."))
    return _mac(dst) + _mac(src_mac) + struct.pack("!H", 0x0806) + body


def _capture(tmp_path: Path, frames: list[bytes], name: str = "hosts.pcap") -> Path:
    cap = pcapgen.Capture()
    for frame in frames:
        cap.clock += 0.001
        cap.frames.append((cap.clock, frame))
    return cap.write_pcap(tmp_path / name)


def _udp(src: str, dst: str, sport: int = 4000, dport: int = 5000) -> bytes:
    return pcapgen.udp(src, sport, dst, dport, b"x")


def _exchange(a: tuple[str, str], b: tuple[str, str], vlan: tuple[int, ...] = ()) -> list[bytes]:
    """One request and one reply between two (ip, mac) hosts."""
    return [
        _frame(a[1], _udp(a[0], b[0]), b[1], vlan),
        _frame(b[1], _udp(b[0], a[0], 5000, 4000), a[1], vlan),
    ]


# Labels


def test_the_labels_section_defaults_to_no_expectation() -> None:
    doc = {"labels": "fps-realism-labels", "version": 1, "hosts": {}}
    assert parse(doc).hosts == HostsLabels(count=None, gateway_macs=())


def test_the_labels_section_reads_a_count_and_normalizes_gateway_macs() -> None:
    doc = {
        "labels": "fps-realism-labels",
        "version": 1,
        "hosts": {"count": {"min": 3, "max": 5}, "gateway_macs": ["02-00-00-00-00-FE", GATEWAY]},
    }
    assert parse(doc).hosts == HostsLabels(count=Count(3, 5), gateway_macs=(GATEWAY,))


@pytest.mark.parametrize(
    "section",
    [
        {"count": -1},
        {"count": True},
        {"count": {"min": 5, "max": 2}},
        {"gateway_macs": ["02:00:00:00:00"]},
        {"gateway_macs": "02:00:00:00:00:fe"},
        {"hosts": 3},
        [],
    ],
)
def test_the_labels_section_rejects_a_malformed_value(section: Any) -> None:
    doc = {"labels": "fps-realism-labels", "version": 1, "hosts": section}
    with pytest.raises(LabelsError):
        parse(doc)


# Counting


def test_a_clean_capture_has_one_host_per_ip_and_no_conflict(tmp_path: Path) -> None:
    frames = _exchange(("10.0.0.1", MAC_A), ("10.0.1.1", MAC_B))
    frames += _exchange(("10.0.0.2", MAC_C), ("10.0.1.1", MAC_B))
    result = hosts.check(_capture(tmp_path, frames))
    assert result.passed
    assert (result.hosts, result.senders, result.macs) == (3, 3, 3)
    assert (result.frames, result.frames_with_mac) == (4, 4)
    assert result.conflicts == []


def test_a_destination_that_never_sends_is_a_host_but_not_a_sender(tmp_path: Path) -> None:
    frames = [_frame(MAC_A, _udp("10.0.0.1", "10.0.0.9"))]
    result = hosts.check(_capture(tmp_path, frames))
    assert (result.hosts, result.senders) == (2, 1)


def test_the_expected_count_decides(tmp_path: Path) -> None:
    pcap = _capture(tmp_path, _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B)))
    assert hosts.check(pcap, expected=Count(2, 2)).passed
    assert hosts.check(pcap, HostsLabels(count=Count(1, 5))).passed
    assert hosts.check(pcap, HostsLabels(count=Count(2, None))).passed
    few = hosts.check(pcap, expected=Count(3, 3))
    assert not few.passed
    assert few.failures == ["the capture holds 2 hosts, the labels expect 3"]
    many = hosts.check(pcap, HostsLabels(count=Count(0, 1)))
    assert many.failures == ["the capture holds 2 hosts, the labels expect 0 to 1"]
    assert hosts.check(pcap, HostsLabels(count=Count(5, None))).failures == [
        "the capture holds 2 hosts, the labels expect at least 5"
    ]


def test_the_count_argument_replaces_the_count_of_the_labels(tmp_path: Path) -> None:
    pcap = _capture(tmp_path, _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B)))
    result = hosts.check(pcap, HostsLabels(count=Count(9, 9)), Count(2, 2))
    assert result.passed
    assert result.expected == Count(2, 2)


def test_multicast_broadcast_and_unspecified_addresses_are_no_hosts(tmp_path: Path) -> None:
    frames = [
        _frame(MAC_A, _udp("10.0.0.1", "255.255.255.255"), BROADCAST),
        _frame(MAC_A, _udp("10.0.0.1", "224.0.0.251"), "01:00:5e:00:00:fb"),
        _frame(MAC_A, _udp("0.0.0.0", "10.0.0.1")),
        _frame(MAC_A, _udp("fe80::1", "ff02::fb"), "33:33:00:00:00:fb"),
        _frame(MAC_A, _udp("::", "fe80::1")),
    ]
    result = hosts.check(_capture(tmp_path, frames))
    assert result.passed
    assert result.hosts == 2  # 10.0.0.1 and fe80::1
    assert result.conflicts == []


def test_an_address_in_two_vlans_is_two_hosts_and_no_conflict(tmp_path: Path) -> None:
    frames = _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B), vlan=(10,))
    frames += _exchange(("10.0.0.1", MAC_C), ("10.0.0.2", MAC_B), vlan=(20,))
    result = hosts.check(_capture(tmp_path, frames))
    assert result.passed
    assert result.hosts == 4
    assert result.conflicts == []


def test_a_double_tagged_frame_keeps_both_tags(tmp_path: Path) -> None:
    frames = _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B), vlan=(10, 5))
    frames += [_frame(MAC_C, _udp("10.0.0.1", "10.0.0.2"), MAC_B, vlan=(10, 5))]
    result = hosts.check(_capture(tmp_path, frames))
    assert [c.vlan for c in result.conflicts] == ["10.5"]


# Conflicts


def test_two_mac_addresses_for_one_ip_address_are_a_conflict(tmp_path: Path) -> None:
    frames = _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B))
    frames += [_frame(MAC_C, _udp("10.0.0.1", "10.0.0.2"), MAC_B)]
    result = hosts.check(_capture(tmp_path, frames))
    assert not result.passed
    assert result.failures == [
        f"address conflict: 10.0.0.1 sends from 2 MAC addresses ({MAC_A} in 1, {MAC_C} in 1 frames)"
    ]
    entry = result.to_dict()
    assert entry["conflicts"] == [
        {
            "ip": "10.0.0.1",
            "vlan": None,
            "macs": [{"mac": MAC_A, "frames": 1}, {"mac": MAC_C, "frames": 1}],
        }
    ]
    assert (entry["hosts"], entry["senders"], entry["macs"]) == (2, 2, 3)
    assert entry["conflicts_total"] == entry["failures_total"] == 1


def test_a_conflict_names_the_vlan_and_ranks_the_busiest_mac_first(tmp_path: Path) -> None:
    frames = [_frame(MAC_B, _udp("10.0.0.1", "10.0.0.9"), vlan=(7,))]
    frames += [_frame(MAC_A, _udp("10.0.0.1", "10.0.0.9"), vlan=(7,)) for _ in range(3)]
    result = hosts.check(_capture(tmp_path, frames))
    assert result.failures == [
        f"address conflict: 10.0.0.1 in VLAN 7 sends from 2 MAC addresses "
        f"({MAC_A} in 3, {MAC_B} in 1 frames)"
    ]


def test_an_ipv6_address_with_two_macs_is_a_conflict(tmp_path: Path) -> None:
    frames = [
        _frame(MAC_A, _udp("2001:db8::1", "2001:db8::2")),
        _frame(MAC_B, _udp("2001:db8::1", "2001:db8::2")),
    ]
    result = hosts.check(_capture(tmp_path, frames))
    assert [c.ip for c in result.conflicts] == ["2001:db8::1"]


def test_one_mac_with_many_ip_addresses_is_no_conflict(tmp_path: Path) -> None:
    frames = [_frame(MAC_A, _udp(f"10.0.0.{n}", "10.0.1.1")) for n in range(1, 21)]
    result = hosts.check(_capture(tmp_path, frames))
    assert result.passed
    assert (result.hosts, result.macs) == (21, 1)


def test_gateway_macs_are_left_out_of_the_conflict_search(tmp_path: Path) -> None:
    frames = _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B))
    frames += [_frame(GATEWAY, _udp("10.0.0.1", "10.0.0.2"), MAC_B)]
    pcap = _capture(tmp_path, frames)
    assert not hosts.check(pcap).passed
    result = hosts.check(pcap, HostsLabels(gateway_macs=(GATEWAY,)))
    assert result.passed
    assert (result.hosts, result.macs, result.frames_with_mac) == (2, 2, 3)


def test_arp_senders_take_part_in_the_conflict_search(tmp_path: Path) -> None:
    frames = _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B))
    frames += [_arp(MAC_C, MAC_C, "10.0.0.2", "10.0.0.1")]
    result = hosts.check(_capture(tmp_path, frames))
    assert [c.ip for c in result.conflicts] == ["10.0.0.2"]
    assert sorted(result.conflicts[0].macs) == [MAC_B, MAC_C]


def test_an_arp_probe_and_the_target_of_a_request_are_not_conflicts(tmp_path: Path) -> None:
    frames = _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B))
    frames += [_arp(MAC_C, MAC_C, "0.0.0.0", "10.0.0.1")]  # a probe for 10.0.0.1
    frames += [_arp(MAC_C, MAC_C, "10.0.0.3", "10.0.0.1")]  # a request that names 10.0.0.1
    result = hosts.check(_capture(tmp_path, frames))
    assert result.passed
    assert result.hosts == 3  # the probe adds no host, the sender 10.0.0.3 does


def test_conflicts_are_listed_in_address_order_and_capped(tmp_path: Path) -> None:
    frames: list[bytes] = []
    for n in (200, 3, 100):
        frames.append(_frame(MAC_A, _udp(f"10.0.0.{n}", "10.0.9.9")))
        frames.append(_frame(MAC_B, _udp(f"10.0.0.{n}", "10.0.9.9")))
    result = hosts.check(_capture(tmp_path, frames))
    assert [c.ip for c in result.conflicts] == ["10.0.0.3", "10.0.0.100", "10.0.0.200"]
    many: list[bytes] = []
    for n in range(hosts.MAX_CONFLICTS + 20):
        many.append(_frame(MAC_A, _udp(f"10.1.{n // 250}.{n % 250 + 1}", "10.0.9.9")))
        many.append(_frame(MAC_B, _udp(f"10.1.{n // 250}.{n % 250 + 1}", "10.0.9.9")))
    entry = hosts.check(_capture(tmp_path, many, "many.pcap")).to_dict()
    assert entry["conflicts_total"] == hosts.MAX_CONFLICTS + 20
    assert len(entry["conflicts"]) == hosts.MAX_CONFLICTS
    assert len(entry["failures"]) == hosts.MAX_FAILURES
    assert entry["failures_total"] == hosts.MAX_CONFLICTS + 20


# Nothing to judge


def _raw_capture(tmp_path: Path) -> Path:
    packet = _udp("10.0.0.1", "10.0.0.2")
    header = struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, LINKTYPE_RAW)
    record = struct.pack("<IIII", 1, 0, len(packet), len(packet)) + packet
    path = tmp_path / "raw.pcap"
    path.write_bytes(header + record)
    return path


def test_a_capture_without_mac_addresses_counts_hosts_but_fails(tmp_path: Path) -> None:
    result = hosts.check(_raw_capture(tmp_path), expected=Count(2, 2))
    assert result.hosts == 2
    assert result.failures == [
        "no frame of the capture has an Ethernet source address, "
        "so address conflicts are not judged"
    ]


def test_a_capture_without_ip_packets_fails(tmp_path: Path) -> None:
    result = hosts.check(_capture(tmp_path, []))
    assert result.failures == [
        "the capture holds no IP host, so the hosts are not judged",
        "no frame of the capture has an Ethernet source address, "
        "so address conflicts are not judged",
    ]


def test_a_multicast_source_address_is_no_station(tmp_path: Path) -> None:
    result = hosts.check(
        _capture(tmp_path, [_frame("01:00:5e:00:00:01", _udp("10.0.0.1", "10.0.0.2"))])
    )
    assert result.macs == 0
    assert not result.passed


def test_truncated_frames_are_skipped(tmp_path: Path) -> None:
    frames = _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B))
    frames += [b"\x00" * 10, _frame(MAC_A, _udp("10.0.0.1", "10.0.0.2"))[:20]]
    result = hosts.check(_capture(tmp_path, frames))
    assert result.passed
    assert result.frames == 4


# The report


def _validate(report: dict[str, Any]) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator(schema).validate(report)


def _doc(section: dict[str, Any] | None) -> dict[str, Any]:
    doc: dict[str, Any] = {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [{"application": "udp", "match": {"transport": "udp"}}],
        "applications": {"udp": {"stub": ["X"]}},
    }
    if section is not None:
        doc["hosts"] = section
    return doc


def test_the_entry_of_a_passing_and_a_failing_check_validates(tmp_path: Path) -> None:
    clean = _capture(tmp_path, _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B)))
    bad = _capture(
        tmp_path,
        [
            *_exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B)),
            _frame(MAC_C, _udp("10.0.0.1", "10.0.0.2")),
        ],
        "bad.pcap",
    )
    for pcap, passed in ((clean, True), (bad, False)):
        entry = hosts.check(pcap, expected=Count(2, 2)).to_dict()
        assert entry["passed"] is passed
        assert entry["check"] == "host_identity"
        assert entry["metrics"] == []
        assert entry["expected_hosts"] == {"min": 2, "max": 2}
        report = {
            "report": "fps-realism-report",
            "version": REPORT_VERSION,
            "generator": "test",
            "passed": passed,
            "capture": {
                "file": pcap.name,
                "sha256": "0" * 64,
                "format": "pcap",
                "link_types": [1],
                "frames": entry["frames"],
                "skipped_frames": 0,
                "sessions": 2,
            },
            "labels": {"file": "l.json", "sha256": "0" * 64},
            "checks": [entry],
        }
        _validate(report)


def test_an_open_expectation_validates_as_null(tmp_path: Path) -> None:
    entry = hosts.check(
        _capture(tmp_path, _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B)))
    ).to_dict()
    assert entry["expected_hosts"] is None
    open_range = hosts.check(
        _capture(tmp_path, _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B))),
        HostsLabels(count=Count(1, None)),
    ).to_dict()
    assert open_range["expected_hosts"] == {"min": 1, "max": None}


def test_the_schema_refuses_a_conflict_with_one_mac() -> None:
    entry = {
        "check": "host_identity",
        "metrics": [],
        "passed": False,
        "expected_hosts": None,
        "hosts": 1,
        "senders": 1,
        "macs": 1,
        "frames": 1,
        "frames_with_mac": 1,
        "conflicts_total": 1,
        "conflicts": [{"ip": "10.0.0.1", "vlan": None, "macs": [{"mac": MAC_A, "frames": 1}]}],
        "failures_total": 1,
        "failures": ["x"],
    }
    report = {
        "report": "fps-realism-report",
        "version": REPORT_VERSION,
        "generator": "test",
        "passed": False,
        "capture": {
            "file": "x.pcap",
            "sha256": "0" * 64,
            "format": "pcap",
            "link_types": [1],
            "frames": 1,
            "skipped_frames": 0,
            "sessions": 1,
        },
        "labels": {"file": "l.json", "sha256": "0" * 64},
        "checks": [entry],
    }
    with pytest.raises(jsonschema.ValidationError):
        _validate(report)


def _two_hosts(tmp_path: Path, conflict: bool) -> Path:
    frames = _exchange(("10.0.0.1", MAC_A), ("10.0.0.2", MAC_B))
    if conflict:
        frames.append(_frame(MAC_C, _udp("10.0.0.1", "10.0.0.2")))
    return _capture(tmp_path, frames)


def test_score_adds_the_check_for_the_section_the_flag_or_the_count(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    pcap = _two_hosts(tmp_path, conflict=False)
    classifier = StubClassifier(lambda _session: "X")
    plain = score(pcap, labels_file(_doc(None)), classifier)
    assert [c["check"] for c in plain.checks] == ["conformance"]
    for kwargs, section in (
        ({}, {"count": 2}),
        ({"host_identity": True}, None),
        ({"host_count": 2}, None),
    ):
        report = score(pcap, labels_file(_doc(section)), classifier, **kwargs)
        assert [c["check"] for c in report.checks] == ["conformance", "host_identity"]
        assert report.passed
        assert report.to_dict()["version"] == REPORT_VERSION == "0.14.0"
        _validate(report.to_dict())
    wrong = score(pcap, labels_file(_doc({"count": 2})), classifier, host_count=3)
    assert not wrong.passed
    assert wrong.checks[1]["failures"] == ["the capture holds 2 hosts, the labels expect 3"]


def test_score_fails_for_a_conflict(tmp_path: Path, labels_file: Callable[..., Path]) -> None:
    pcap = _two_hosts(tmp_path, conflict=True)
    report = score(pcap, labels_file(_doc({})), StubClassifier(lambda _session: "X"))
    assert not report.passed
    assert report.checks[1]["conflicts_total"] == 1
    _validate(report.to_dict())


def _host_entry(out: str) -> dict[str, Any]:
    report = json.loads(out)
    return next(c for c in report["checks"] if c["check"] == "host_identity")


def test_the_command_line_flags_run_the_check(
    tmp_path: Path,
    fake_reader: Callable[..., Path],
    labels_file: Callable[..., Path],
    capsys: pytest.CaptureFixture[str],
) -> None:
    argv = ["score", "--labels", str(labels_file()), "--parsers", "none"]
    argv += ["--ndpi-reader", str(fake_reader())]
    clean = _two_hosts(tmp_path, conflict=False)
    main([*argv, "--pcap", str(clean), "--host-count", "2"])
    entry = _host_entry(capsys.readouterr().out)
    assert entry["passed"] is True
    assert entry["expected_hosts"] == {"min": 2, "max": 2}
    main([*argv, "--pcap", str(clean), "--host-count", "5"])
    entry = _host_entry(capsys.readouterr().out)
    assert entry["failures"] == ["the capture holds 2 hosts, the labels expect 5"]
    bad = _two_hosts(tmp_path, conflict=True)
    assert main([*argv, "--pcap", str(bad), "--host-identity"]) == EXIT_FAIL
    entry = _host_entry(capsys.readouterr().out)
    assert entry["conflicts_total"] == 1
    assert entry["expected_hosts"] is None


def test_the_command_line_refuses_a_negative_host_count(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    argv = ["score", "--pcap", str(_two_hosts(tmp_path, False)), "--labels", str(labels_file())]
    with pytest.raises(SystemExit) as stop:
        main([*argv, "--host-count", "-1"])
    assert stop.value.code == 2
