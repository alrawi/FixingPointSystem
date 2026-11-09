# SPDX-License-Identifier: GPL-3.0-only
"""Group decoded packets into TCP sessions and UDP flows, and render session JSON."""

from __future__ import annotations

import base64
from collections import Counter
from collections.abc import Iterable
from dataclasses import dataclass, field
from typing import Any

from . import SCHEMA_VERSION
from .decode import TCP_ACK, TCP_FIN, TCP_RST, TCP_SYN, Packet, address
from .pcapio import Interface
from .tcp import Stream

DEFAULT_UDP_IDLE_S = 60.0
_C2S, _S2C = 0, 1
_DIRS = ("client_to_server", "server_to_client")
_NS_PER_MS = 1_000_000


def _ms(ns: int) -> float:
    return round(ns / _NS_PER_MS, 3)


@dataclass
class _Side:
    packets: int = 0
    ip_bytes: int = 0
    payload_bytes: int = 0


@dataclass
class _Session:
    protocol: str
    version: int
    client: tuple[bytes, int]
    server: tuple[bytes, int]
    basis: str
    start_ns: int
    keep_payload: bool
    continues: int | None = None
    index: int = 0
    end_ns: int = 0
    end_reason: str = "capture_end"
    sides: tuple[_Side, _Side] = field(default_factory=lambda: (_Side(), _Side()))
    streams: tuple[Stream, Stream] = field(init=False)
    interfaces: set[int] = field(default_factory=set)
    syn_ns: int | None = None
    synack_ns: int | None = None
    ack_ns: int | None = None
    syn_count: int = 0
    server_isn: int | None = None
    client_isn: int | None = None
    data_start: int | None = None
    data_end: int | None = None
    truncated: bool = False
    closed: bool = False
    datagrams: list[tuple[int, int, int, bytes]] = field(default_factory=list)

    def __post_init__(self) -> None:
        self.streams = (Stream(self.keep_payload), Stream(self.keep_payload))

    def direction(self, pkt: Packet) -> int:
        """Return 0 for a packet from the client and 1 for one from the server."""
        return _C2S if (pkt.src, pkt.sport) == self.client else _S2C

    def fin_done(self) -> bool:
        """Both directions sent a FIN."""
        return all(s.fin_ts is not None for s in self.streams)

    def rst_done(self) -> bool:
        """Either direction sent a RST."""
        return any(s.rst_ts is not None for s in self.streams)

    def add(self, pkt: Packet) -> None:
        """Account one packet that belongs to this session."""
        d = self.direction(pkt)
        side = self.sides[d]
        side.packets += 1
        side.ip_bytes += pkt.ip_len
        side.payload_bytes += pkt.payload_len
        self.end_ns = max(self.end_ns, pkt.ts_ns)
        self.interfaces.add(pkt.interface)
        self.truncated = self.truncated or pkt.truncated
        if pkt.payload_len:
            self.data_start = pkt.ts_ns if self.data_start is None else self.data_start
            self.data_end = pkt.ts_ns
        if pkt.proto == "udp":
            self.datagrams.append((pkt.ts_ns, d, pkt.payload_len, pkt.payload))
            return
        self._tcp(pkt, d)

    def _tcp(self, pkt: Packet, d: int) -> None:
        stream = self.streams[d]
        syn = bool(pkt.flags & TCP_SYN)
        ack = bool(pkt.flags & TCP_ACK)
        if syn:
            stream.anchor(pkt.seq, syn=True)
            if not ack and d == _C2S:
                self.syn_count += 1
                self.client_isn = pkt.seq
                if self.synack_ns is None:  # the last SYN before the SYN ACK measures the RTT
                    self.syn_ns = pkt.ts_ns
            elif ack and d == _S2C and self.synack_ns is None:
                self.synack_ns = pkt.ts_ns
                self.server_isn = pkt.seq
        elif (
            ack
            and d == _C2S
            and self.ack_ns is None
            and self.synack_ns is not None
            and self.server_isn is not None
            and pkt.ack == (self.server_isn + 1) % (1 << 32)
        ):
            self.ack_ns = pkt.ts_ns
        if pkt.payload_len:
            stream.add(pkt.ts_ns, pkt.seq + (1 if syn else 0), pkt.payload_len, pkt.payload)
        if pkt.flags & TCP_FIN and stream.fin_ts is None:
            stream.fin_ts = pkt.ts_ns
        if pkt.flags & TCP_RST and stream.rst_ts is None:
            stream.rst_ts = pkt.ts_ns
        self.closed = self.fin_done() or self.rst_done()

    # -- rendering ---------------------------------------------------------

    def handshake(self) -> dict[str, Any] | None:
        """The handshake record of a TCP session."""
        if self.protocol != "tcp":
            return None
        has_syn = self.syn_ns is not None
        complete = has_syn and self.synack_ns is not None and self.ack_ns is not None
        if complete:
            state = "complete"
        elif has_syn or self.synack_ns is not None:
            state = "incomplete"
        else:
            state = "missing"
        out: dict[str, Any] = {
            "state": state,
            "server_rtt_ms": None,
            "client_rtt_ms": None,
            "rtt_ms": None,
        }
        if self.syn_ns is not None and self.synack_ns is not None and self.synack_ns >= self.syn_ns:
            out["server_rtt_ms"] = _ms(self.synack_ns - self.syn_ns)
        if self.synack_ns is not None and self.ack_ns is not None and self.ack_ns >= self.synack_ns:
            out["client_rtt_ms"] = _ms(self.ack_ns - self.synack_ns)
        if out["server_rtt_ms"] is not None and out["client_rtt_ms"] is not None:
            out["rtt_ms"] = round(out["server_rtt_ms"] + out["client_rtt_ms"], 3)
        return out

    def anomalies(self, hs: dict[str, Any] | None, client_known: bool) -> list[str]:
        """The sorted anomaly flags of this session."""
        found: set[str] = set()
        if self.truncated:
            found.add("truncated_payload")
        if not client_known:
            found.add("role_guessed")
        if self.continues is not None:
            found.add("udp_idle_split")
        if self.protocol == "tcp":
            if hs is not None and hs["state"] == "missing":
                found.add("missing_handshake")
            elif hs is not None and hs["state"] == "incomplete":
                found.add("handshake_incomplete")
            if self.syn_count > 1:
                found.add("syn_retransmitted")
            for s in self.streams:
                if s.retrans_packets:
                    found.add("retransmission")
                if s.out_of_order_packets:
                    found.add("out_of_order")
                if s.mismatch_packets:
                    found.add("overlap_mismatch")
                if s.missing_bytes or s.skipped_bytes:
                    found.add("data_gap")
                if s.overflowed:
                    found.add("reassembly_overflow")
            if self.rst_done():
                found.add("reset")
            elif any(s.fin_ts is not None for s in self.streams) and not self.fin_done():
                found.add("half_close")
        return sorted(found)

    def render(self, sid: int, include_payload: bool) -> dict[str, Any]:
        """Return the JSON object of this session."""
        for s in self.streams:
            s.finish()
        hs = self.handshake()
        client_known = self.basis in ("syn", "synack")
        if self.protocol == "tcp":
            if self.rst_done():
                self.end_reason = "rst"
            elif self.fin_done():
                self.end_reason = "fin"
            elif any(s.fin_ts is not None for s in self.streams):
                self.end_reason = "half_close"
        sides: dict[str, Any] = {}
        for d, name in enumerate(_DIRS):
            side, stream = self.sides[d], self.streams[d]
            row: dict[str, Any] = {
                "packets": side.packets,
                "ip_bytes": side.ip_bytes,
                "payload_bytes": side.payload_bytes,
            }
            if self.protocol == "tcp":
                row.update(
                    stream_bytes=stream.unique_bytes,
                    retransmitted_packets=stream.retrans_packets,
                    retransmitted_bytes=stream.retrans_bytes,
                    out_of_order_packets=stream.out_of_order_packets,
                    missing_bytes=stream.missing_bytes,
                    fin=stream.fin_ts is not None,
                    rst=stream.rst_ts is not None,
                )
            sides[name] = row
        return {
            "id": sid,
            "protocol": self.protocol,
            "ip_version": self.version,
            "client": {"ip": address(self.client[0]), "port": self.client[1]},
            "server": {"ip": address(self.server[0]), "port": self.server[1]},
            "roles": {"client_known": client_known, "basis": self.basis},
            "interfaces": sorted(self.interfaces),
            "start_ns": self.start_ns,
            "end_ns": self.end_ns,
            "duration_ms": _ms(self.end_ns - self.start_ns),
            "data_start_ns": self.data_start,
            "data_end_ns": self.data_end,
            "data_duration_ms": (
                None
                if self.data_start is None or self.data_end is None
                else _ms(self.data_end - self.data_start)
            ),
            "end_reason": self.end_reason,
            "continues": self.continues,
            "handshake": hs,
            **sides,
            "anomalies": self.anomalies(hs, client_known),
            "turns": self._turns(include_payload),
        }

    def _turns(self, include_payload: bool) -> list[dict[str, Any]]:
        """Merge deliveries in time order into runs of one direction."""
        events: list[tuple[int, int, int, bytes | None, int]]
        if self.protocol == "udp":
            events = [(ts, d, n, data, 0) for ts, d, n, data in self.datagrams]
        else:
            events = [
                (e.ts_ns, d, e.length, e.data, e.gap_before)
                for d in (_C2S, _S2C)
                for e in self.streams[d].deliveries
            ]
            events.sort(key=lambda e: (e[0], e[1]))  # stable, so offset order survives
        turns: list[dict[str, Any]] = []
        chunks: list[list[bytes | None]] = []
        for ts, d, n, data, gap in events:
            last = turns[-1] if turns else None
            if last is None or last["direction"] != _DIRS[d] or gap:
                last = {
                    "direction": _DIRS[d],
                    "start_ns": ts,
                    "end_ns": ts,
                    "bytes": 0,
                    "packets": 0,
                    "gap_before": gap,
                }
                turns.append(last)
                chunks.append([])
            last["end_ns"] = ts
            last["bytes"] += n
            last["packets"] += 1
            chunks[-1].append(data)
            if self.protocol == "udp":
                last.setdefault("datagram_sizes", []).append(n)
        for turn, parts in zip(turns, chunks, strict=True):
            if not include_payload:
                continue
            blobs = [p if p is not None else b"" for p in parts]
            if self.protocol == "udp":
                turn["datagrams_b64"] = [base64.b64encode(b).decode("ascii") for b in blobs]
            else:
                turn["data_b64"] = base64.b64encode(b"".join(blobs)).decode("ascii")
        return turns


def _flow_key(pkt: Packet) -> tuple[Any, ...]:
    a, b = (pkt.src, pkt.sport), (pkt.dst, pkt.dport)
    lo, hi = (a, b) if a <= b else (b, a)
    return (pkt.proto, pkt.version, lo, hi)


def _roles(pkt: Packet) -> tuple[tuple[bytes, int], tuple[bytes, int], str]:
    """Choose client and server of a session from its first packet."""
    src, dst = (pkt.src, pkt.sport), (pkt.dst, pkt.dport)
    if pkt.proto == "tcp":
        if pkt.flags & TCP_SYN and not pkt.flags & TCP_ACK:
            return src, dst, "syn"
        if pkt.flags & TCP_SYN:
            return dst, src, "synack"
    if pkt.sport < 1024 <= pkt.dport:
        return dst, src, "port"
    if pkt.dport < 1024 <= pkt.sport:
        return src, dst, "port"
    return src, dst, "first_packet"


def _starts_new(sess: _Session, pkt: Packet) -> bool:
    """Whether a TCP packet starts a connection that the session does not hold."""
    if pkt.proto != "tcp":
        return False
    syn, ack = bool(pkt.flags & TCP_SYN), bool(pkt.flags & TCP_ACK)
    d = sess.direction(pkt)
    if syn and not ack:
        if sess.client_isn is not None:
            return pkt.seq != sess.client_isn
        return sess.closed or d == _S2C or sess.basis != "synack"
    if syn and ack:
        if sess.server_isn is not None:
            return pkt.seq != sess.server_isn
        return sess.closed or d == _C2S or sess.basis != "syn"
    return False


class Extractor:
    """Feed packets in time order, then read the sessions."""

    def __init__(self, udp_idle_s: float = DEFAULT_UDP_IDLE_S, *, payloads: bool = False) -> None:
        self.idle_ns = int(udp_idle_s * 1_000_000_000)
        self.payloads = payloads
        self._open: dict[tuple[Any, ...], _Session] = {}
        self._all: list[_Session] = []

    def add(self, pkt: Packet) -> None:
        """Place one packet in a session, and start a new session when the rules say so."""
        key = _flow_key(pkt)
        sess = self._open.get(key)
        if sess is not None and pkt.proto == "udp" and pkt.ts_ns - sess.end_ns > self.idle_ns:
            sess.end_reason = "idle_timeout"
            sess = self._begin(pkt, previous=sess)
        elif sess is not None and _starts_new(sess, pkt):
            if not sess.closed:
                sess.end_reason = "new_connection"
            sess = self._begin(pkt, previous=None)
        elif sess is None:
            sess = self._begin(pkt, previous=None)
        sess.add(pkt)

    def _begin(self, pkt: Packet, previous: _Session | None) -> _Session:
        if previous is not None:
            client, server, basis = previous.client, previous.server, previous.basis
        else:
            client, server, basis = _roles(pkt)
        sess = _Session(
            pkt.proto,
            pkt.version,
            client,
            server,
            basis,
            pkt.ts_ns,
            self.payloads,
            continues=None if previous is None else previous.index,
        )
        sess.index = len(self._all) + 1
        self._open[_flow_key(pkt)] = sess
        self._all.append(sess)
        return sess

    def sessions(self) -> list[dict[str, Any]]:
        """Render every session, ordered by start time and then by arrival."""
        ordered = sorted(self._all, key=lambda s: (s.start_ns, s.index))
        ids = {s.index: n for n, s in enumerate(ordered, 1)}
        out = []
        for sess in ordered:
            if sess.continues is not None:
                sess.continues = ids[sess.continues]
            out.append(sess.render(ids[sess.index], self.payloads))
        return out


def build_document(
    packets: Iterable[Packet],
    *,
    source: dict[str, Any],
    interfaces: list[Interface],
    udp_idle_s: float = DEFAULT_UDP_IDLE_S,
    payloads: bool = False,
) -> dict[str, Any]:
    """Return the session document for decoded packets in time order."""
    extractor = Extractor(udp_idle_s, payloads=payloads)
    for pkt in packets:
        extractor.add(pkt)
    sessions = extractor.sessions()
    counts = Counter(s["protocol"] for s in sessions)
    source = dict(source)
    source["interfaces"] = [
        {
            "id": i.id,
            "link_type": i.link_type,
            "snaplen": i.snaplen,
            "name": i.name,
            "tsresol": i.tsresol,
            "section": i.section,
        }
        for i in interfaces
    ]
    return {
        "schema_version": SCHEMA_VERSION,
        "source": source,
        "parameters": {"udp_idle_timeout_s": udp_idle_s, "payloads": payloads},
        "summary": {
            "sessions": len(sessions),
            "tcp": counts.get("tcp", 0),
            "udp": counts.get("udp", 0),
        },
        "sessions": sessions,
    }
