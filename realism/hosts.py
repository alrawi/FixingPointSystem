# SPDX-License-Identifier: GPL-3.0-only
"""Host identity check of the realism runner.

A generated capture can reach the configured number of sessions and still show a population
that no real network has: too few hosts, or two machines that claim one IP address. This
check counts the hosts of a capture and looks for address conflicts.

* A host is an IP address that appears as the source or the destination of an IP packet, or as
  the sender of an ARP packet, within one VLAN. The same address in two VLANs is two hosts.
  Multicast, broadcast, reserved, and unspecified addresses are not hosts. A machine with an
  IPv4 and an IPv6 address counts twice, because the capture does not say that they belong
  together.
* An address conflict is an IP address that sends frames with more than one Ethernet source
  address in one VLAN. The sources are IPv4 and IPv6 packets and ARP senders (the sender
  fields of requests and replies, and of announcements, but not an ARP probe, which has the
  sender address 0.0.0.0). A router forwards packets of other addresses with its own MAC
  address, which is one MAC for many IPs and never a conflict, but a capture on both sides of
  a router shows the forwarded addresses with two MACs, so the labels may list the
  ``gateway_macs`` whose frames the conflict search leaves out.

The check reads Ethernet frames, with VLAN tags. Frames of other link types carry no MAC
address and count only toward the hosts. A capture without any Ethernet source address fails,
because a skipped judgement never passes.
"""

from __future__ import annotations

import ipaddress
import struct
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from realism.capture import LINKTYPE_ETHERNET, _network_layer, read_frames
from realism.labels import Count, HostsLabels

CHECK_NAME = "host_identity"
MAX_FAILURES = 50
MAX_CONFLICTS = 100

_ETH_IPV4 = 0x0800
_ETH_IPV6 = 0x86DD
_ETH_ARP = 0x0806
_ETH_VLAN = (0x8100, 0x88A8, 0x9100)
_ARP_ETHERNET_IPV4 = struct.pack("!HHBB", 1, _ETH_IPV4, 6, 4)

# A host key is the VLAN tag stack (empty when untagged) and the packed IP address, whose
# length of 4 or 16 bytes tells the IP version.
_HostKey = tuple[tuple[int, ...], bytes]


@dataclass
class Conflict:
    """One IP address that sent frames from more than one MAC address."""

    ip: str
    vlan: str | None
    macs: dict[str, int]

    def to_dict(self) -> dict[str, Any]:
        """Return the conflict as the report shows it, the MAC with most frames first."""
        ranked = sorted(self.macs.items(), key=lambda item: (-item[1], item[0]))
        return {
            "ip": self.ip,
            "vlan": self.vlan,
            "macs": [{"mac": mac, "frames": frames} for mac, frames in ranked],
        }

    def sentence(self) -> str:
        """Describe the conflict in words."""
        ranked = sorted(self.macs.items(), key=lambda item: (-item[1], item[0]))
        seen = ", ".join(f"{mac} in {frames}" for mac, frames in ranked[:4])
        more = f" and {len(ranked) - 4} more" if len(ranked) > 4 else ""
        where = f" in VLAN {self.vlan}" if self.vlan is not None else ""
        return (
            f"address conflict: {self.ip}{where} sends from {len(ranked)} MAC addresses "
            f"({seen}{more} frames)"
        )


@dataclass
class HostsResult:
    """The outcome of the host identity check."""

    expected: Count | None
    frames: int = 0
    frames_with_mac: int = 0
    hosts: int = 0
    senders: int = 0
    macs: int = 0
    conflicts: list[Conflict] = field(default_factory=list)
    failures: list[str] = field(default_factory=list)

    @property
    def passed(self) -> bool:
        """True when no reason in ``failures`` stands against the capture."""
        return not self.failures

    def to_dict(self) -> dict[str, Any]:
        """Return the check entry of the realism report."""
        return {
            "check": CHECK_NAME,
            # The catalog label `check` has no value `host_identity` (catalog 0.6.0), so the check
            # carries no realism.check_failures series until a catalog change adds one.
            "metrics": [],
            "passed": self.passed,
            "expected_hosts": self.expected.to_dict() if self.expected is not None else None,
            "hosts": self.hosts,
            "senders": self.senders,
            "macs": self.macs,
            "frames": self.frames,
            "frames_with_mac": self.frames_with_mac,
            "conflicts_total": len(self.conflicts),
            "conflicts": [c.to_dict() for c in self.conflicts[:MAX_CONFLICTS]],
            "failures_total": len(self.failures),
            "failures": self.failures[:MAX_FAILURES],
        }


def _usable_ip(packed: bytes) -> bool:
    """Return False for the unspecified, multicast, broadcast, and reserved addresses."""
    if len(packed) == 4:
        return packed != b"\x00\x00\x00\x00" and packed[0] < 224
    return packed != bytes(16) and packed[0] != 0xFF


def _vlan_text(tags: tuple[int, ...]) -> str | None:
    return ".".join(str(tag) for tag in tags) if tags else None


def _packet_addresses(ethertype: int, body: bytes) -> tuple[bytes, bytes] | None:
    """Return the packed source and destination of an IPv4 or IPv6 packet."""
    if ethertype == _ETH_IPV4 and len(body) >= 20 and body[0] >> 4 == 4:
        return body[12:16], body[16:20]
    if ethertype == _ETH_IPV6 and len(body) >= 40 and body[0] >> 4 == 6:
        return body[8:24], body[24:40]
    return None


def _arp_sender(body: bytes) -> bytes | None:
    """Return the sender IPv4 address of an ARP request or reply, or None for anything else."""
    if len(body) < 28 or body[:6] != _ARP_ETHERNET_IPV4:
        return None
    if struct.unpack("!H", body[6:8])[0] not in (1, 2):
        return None
    return body[14:18]


def check(
    capture: Path,
    settings: HostsLabels | None = None,
    expected: Count | None = None,
) -> HostsResult:
    """Count the hosts of a capture and look for IP addresses with more than one MAC address.

    ``settings`` is the ``hosts`` section of the labels, if any, and ``expected`` replaces its
    host count.
    """
    settings = settings if settings is not None else HostsLabels()
    if expected is None:
        expected = settings.count
    gateways = {bytes.fromhex(mac.replace(":", "")) for mac in settings.gateway_macs}
    result = HostsResult(expected)
    hosts: set[_HostKey] = set()
    senders: set[_HostKey] = set()
    bindings: dict[_HostKey, Counter[bytes]] = {}
    macs: set[bytes] = set()
    _fmt, frames = read_frames(capture)
    for frame in frames:
        result.frames += 1
        if frame.link_type != LINKTYPE_ETHERNET:
            layer = _network_layer(frame)  # no MAC address, but the IP addresses are hosts
            if layer is not None:
                packet = _packet_addresses(_ETH_IPV4 if layer[0] == 4 else _ETH_IPV6, layer[1])
                for address in packet or ():
                    if _usable_ip(address):
                        hosts.add(((), address))
            continue
        if len(frame.data) < 14:
            continue
        data = frame.data
        src_mac = data[6:12]
        ethertype = struct.unpack("!H", data[12:14])[0]
        pos = 14
        tags: list[int] = []
        while ethertype in _ETH_VLAN:
            if len(data) < pos + 4:
                break
            tci, ethertype = struct.unpack("!HH", data[pos : pos + 4])
            tags.append(tci & 0x0FFF)
            pos += 4
        vlan = tuple(tags)
        body = data[pos:]
        talkers: list[bytes] = []
        packet = _packet_addresses(ethertype, body)
        if packet is not None:
            talkers = [packet[0]]
            for address in packet:
                if _usable_ip(address):
                    hosts.add((vlan, address))
        elif ethertype == _ETH_ARP:
            sender = _arp_sender(body)
            if sender is not None:
                talkers = [sender]
                if _usable_ip(sender):
                    hosts.add((vlan, sender))
        if src_mac[0] & 1 or src_mac == bytes(6):
            continue  # a multicast or empty source address is no station address
        result.frames_with_mac += 1
        if src_mac in gateways:
            continue
        macs.add(src_mac)
        for address in talkers:
            if _usable_ip(address):
                senders.add((vlan, address))
                bindings.setdefault((vlan, address), Counter())[src_mac] += 1
    result.hosts = len(hosts)
    result.senders = len(senders)
    result.macs = len(macs)
    result.conflicts = _conflicts(bindings)
    _decide(result)
    return result


def _conflicts(bindings: dict[_HostKey, Counter[bytes]]) -> list[Conflict]:
    found: list[tuple[tuple[int, int, tuple[int, ...]], Conflict]] = []
    for (vlan, packed), counter in bindings.items():
        if len(counter) < 2:
            continue
        address = ipaddress.ip_address(packed)
        order = (address.version, int(address), vlan)
        names = {":".join(f"{b:02x}" for b in mac): frames for mac, frames in counter.items()}
        found.append((order, Conflict(str(address), _vlan_text(vlan), names)))
    found.sort(key=lambda item: item[0])
    return [conflict for _order, conflict in found]


def _decide(result: HostsResult) -> None:
    """Turn the counts into failure sentences."""
    result.failures.extend(conflict.sentence() for conflict in result.conflicts)
    if result.hosts == 0:
        result.failures.append("the capture holds no IP host, so the hosts are not judged")
    elif result.expected is not None and not result.expected.holds(result.hosts):
        result.failures.append(
            f"the capture holds {result.hosts} hosts, the labels expect {result.expected.text()}"
        )
    if result.frames_with_mac == 0:
        result.failures.append(
            "no frame of the capture has an Ethernet source address, "
            "so address conflicts are not judged"
        )
