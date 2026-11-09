# Capture

This page describes the capture compiler under `capture/`, which the CAP workstream owns. The compiler turns a packet capture into data for scenario generation, scrubbing, and profile extraction. CAP-01 added the first command, `python3 -m capture sessions`, which reassembles TCP streams, groups UDP flows, and writes one JSON document of sessions with timing. The later packages CAP-02 (scenario generation), CAP-03 (scrubbing and provenance), and CAP-04 (profile extractor) read that document, so its fields are the seam between them.

The package needs only the Python standard library, version 3.10 or newer, and it reads captures as untrusted input, as [Coding Standards](Coding-Standards.md) require. The [Realism](Realism.md) runner has its own reader for scoring, and the two do not share code, because a realism check must not depend on the compiler that produced the traffic it judges.

## Command

```bash
python3 -m capture sessions --pcap FILE [--out FILE] [--udp-idle-timeout SECONDS] [--payloads] [--interface ID]
```

| Option | Meaning |
|---|---|
| `--pcap` | Classic pcap in either byte order with microsecond or nanosecond timestamps, or pcapng with any number of sections and interfaces |
| `--out` | Output file, default standard output |
| `--udp-idle-timeout` | A UDP flow idle for more than this many seconds ends. The default is 60 |
| `--payloads` | Adds the payload bytes of every turn as base64. Without it the document holds sizes and times only, which keeps it free of content until CAP-03 scrubs it |
| `--interface` | Reads only the frames of one interface id. Ids count over the whole file, across pcapng sections |

The exit code is 0 on success and 1 when the capture cannot be read. A capture that ends inside a record yields the earlier packets and a warning, which is what a killed `tcpdump` leaves behind.

## Reading captures

| Format | Handling |
|---|---|
| pcap | Four magic numbers (two byte orders, two timestamp units), link type from the file header |
| pcapng | Section header, interface description, enhanced packet, and simple packet blocks. Other blocks are skipped, which includes the decryption secrets blocks that `fps-capture` writes. Each interface applies its own `if_tsresol` (powers of 10 and of 2) and `if_tsoffset`. A section header starts a new section with its own byte order and interface table. Simple packet blocks carry no timestamp, so they count as time 0 and the document warns |
| Link types | Ethernet (with 802.1Q and 802.1ad tags), null, raw IP, Linux cooked v1 and v2, and the IPv4 and IPv6 link types |
| Network | IPv4 and IPv6, with hop by hop, routing, destination options, and authentication extension headers. Fragments and other transport protocols are skipped and counted by reason in `source.skipped_frames` |

Timestamps are integer nanoseconds since the Unix epoch, so no precision is lost to floating point. A capture whose timestamps go backwards, such as the concatenated per agent files that the controller serves, is sorted by time with a stable sort and the document warns. Frames that two agents captured of the same packet are not merged, so extract one agent at a time with `--interface` when both vantage points are in one file, or the duplicates show up as retransmissions.

## Sessions

A session is one TCP connection or one UDP flow. Both directions of a 5-tuple belong to one session. Sessions are ordered by start time and numbered from 1.

**TCP.** A SYN without ACK starts a session and names the client. A SYN ACK as the first packet names the server. Packets that arrive after the connection closed with FIN from both sides or with a RST stay with the session until a new SYN comes. A SYN with a different initial sequence number starts a new session, and it ends an open one with the reason `new_connection`. A SYN with the same number is a retransmission.

**No handshake.** A capture that starts mid connection has no SYN. The session starts at its first packet, and the roles come from the ports: when exactly one port is below 1024, that side is the server. Otherwise the sender of the first packet is the client. Such sessions carry `roles.client_known: false` and the anomaly `role_guessed`.

**UDP.** A flow groups the datagrams of one 5-tuple. A gap longer than the idle timeout ends the session with the reason `idle_timeout`, and the next datagram starts a session with the field `continues` set to the earlier id and the anomaly `udp_idle_split`. The roles carry over from the first session of the flow. A gap of exactly the timeout does not split.

## TCP reassembly

Each direction keeps its own stream. Sequence numbers are unwrapped past 32 bits and counted from the first byte after the SYN, or from the first data segment when the SYN is missing.

| Situation | Handling |
|---|---|
| In order segment | Delivered at once |
| Segment ahead of a hole | Buffered, up to 16 MiB per direction. Delivered when the hole fills, with the time of the segment that filled it |
| Segment that fills a hole | Counted in `out_of_order_packets` |
| Segment whose bytes were all seen | A retransmission. It counts in `retransmitted_packets` and `retransmitted_bytes`, and the stream counts its bytes once |
| Segment that overlaps old and new bytes | Only the new bytes enter the stream, and the old part counts as retransmitted |
| Overlap with different bytes | The first copy wins, and the anomaly `overlap_mismatch` records it. The check compares against buffered data and the last 64 KiB of delivered data |
| Hole that never fills | At the end of the session the buffered bytes are delivered after the hole. `missing_bytes` counts the gap, and the next turn carries it as `gap_before` |
| Segment cut by the snap length | Counted by its wire length and padded with zero bytes. The anomaly `truncated_payload` marks the session |

A hole filled by a retransmission of a segment that the network lost looks the same as reordering to a capture point at the receiver, so the extractor calls both out of order. A capture point at the sender sees the loss as a retransmission. The distinction is a property of the vantage point, and the document does not guess it.

## Handshake and timing

`handshake.state` is `complete` when the capture holds the SYN, the SYN ACK, and the ACK that acknowledges the server's initial sequence number, `incomplete` when it holds some of them, and `missing` when it holds no SYN and no SYN ACK. A retransmitted SYN adds the anomaly `syn_retransmitted`, and the round trip starts from the last SYN before the SYN ACK.

| Field | Meaning |
|---|---|
| `server_rtt_ms` | Last SYN to the SYN ACK: the round trip between the capture point and the server |
| `client_rtt_ms` | SYN ACK to the client ACK: the round trip between the capture point and the client |
| `rtt_ms` | The sum, the handshake time end to end. Null unless the handshake is complete |

A field is null when the capture cannot measure it. `duration_ms` runs from the first to the last packet. `data_start_ns`, `data_end_ns`, and `data_duration_ms` cover the packets that carry payload, so a session that idles after its handshake still has a short data duration. `end_reason` is `fin`, `rst`, `half_close`, `capture_end`, `new_connection`, or `idle_timeout`.

## Per direction counters

Each session has `client_to_server` and `server_to_client` objects. `packets`, `ip_bytes` (the sum of IP total lengths), and `payload_bytes` (transport payload on the wire, retransmitted copies included) exist for both protocols. TCP adds `stream_bytes` (distinct bytes), `retransmitted_packets`, `retransmitted_bytes`, `out_of_order_packets`, `missing_bytes`, `fin`, and `rst`.

## Anomaly flags

| Flag | Meaning |
|---|---|
| `missing_handshake` | No SYN and no SYN ACK in the capture |
| `handshake_incomplete` | Some but not all of the three handshake packets |
| `syn_retransmitted` | More than one SYN before the SYN ACK |
| `retransmission` | At least one segment repeated bytes |
| `out_of_order` | At least one segment filled a hole |
| `overlap_mismatch` | A repeated range differed from the first copy |
| `data_gap` | Bytes are missing in a stream |
| `reassembly_overflow` | The buffer limit forced delivery past a hole |
| `reset` | A RST was seen |
| `half_close` | One direction sent FIN and the other did not |
| `role_guessed` | No SYN or SYN ACK fixed the client |
| `truncated_payload` | The snap length cut at least one segment |
| `udp_idle_split` | The UDP flow continues an earlier session |

## Turns

`turns` is the conversation of the session, which CAP-02 reads to build flow programs. A turn is a run of deliveries in one direction. For TCP a delivery is a stretch of bytes in stream order, and the turn's `start_ns` is the time at which the first bytes became available, so a segment that waited behind a hole takes the time of the segment that released it. For UDP a delivery is a datagram, and `datagram_sizes` keeps the boundaries. A gap in a TCP stream always starts a new turn and records `gap_before`. With `--payloads`, a TCP turn has `data_b64` and a UDP turn has `datagrams_b64`.

## Schema and versioning

[`capture/sessions.schema.json`](https://github.com/alrawi/FixingPointSystem/blob/main/capture/sessions.schema.json) is a JSON Schema (draft 2020-12) of the document, and every test validates its output against it. The document carries `schema_version` `1.0`. CAP-02 to CAP-04 may add optional fields in a minor version. A change that removes or reinterprets a field needs a new major version and an issue that names the packages that read it.

## Tests

`make lint-cap test-cap` runs `ruff format --check`, `ruff check`, `mypy --strict`, and `pytest`. `capture/tests/pcapgen.py` writes pcap and pcapng bytes, so no binary fixture is checked in. The tests cover each rule above: retransmission and overlap, out of order delivery, holes that never fill, missing handshakes and mid stream starts, SYN retransmission, resets and half closes, port reuse, sequence wraparound, IPv6, VLAN tags, snap length cuts, UDP idle splits, pcap variants, pcapng with several interfaces and resolutions, concatenated sections, and damaged files.

CI has no workflow for `capture/` yet. A job needs these steps.

```yaml
- uses: actions/checkout@v4
- uses: actions/setup-python@v5
  with:
    python-version: "3.12"
- run: python -m pip install -r capture/requirements-dev.txt
- run: make lint-cap test-cap PYTHON=python
- run: |
    python -m pip install ./capture
    python -I -m capture sessions --pcap "$RUNNER_TEMP/sample.pcap" --out "$RUNNER_TEMP/sessions.json"
```

The last step needs a sample capture, for example one that a test writes first, and checks that the installed command runs under `python -I`, as the Python rules require for code that reads captures.

## Limits

The extractor loads the decoded packets of a capture in memory, so memory grows with the packet count and the payload size. A run over 100,000 segments of 1,400 bytes took under two seconds in a test run on the development machine. It does not reassemble IP fragments, it ignores TCP options such as SACK and window scaling, and it does not check checksums. A session that spans a UDP idle split is two sessions by design.
