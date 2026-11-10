# Capture

This page describes the capture compiler under `capture/`, which the CAP workstream owns. The compiler turns a packet capture into data for scenario generation, scrubbing, and profile extraction. CAP-01 added the first command, `python3 -m capture sessions`, which reassembles TCP streams, groups UDP flows, and writes one JSON document of sessions with timing. CAP-02 added `python3 -m capture scenario`, which turns that document into a module directory of scenarios. The later packages CAP-03 (scrubbing and provenance), and CAP-04 (profile extractor) read that document, so its fields are the seam between them.

The compiler needs only the Python standard library, version 3.10 or newer (PyYAML is a test dependency, because the validator reads YAML), and it reads captures as untrusted input, as [Coding Standards](Coding-Standards.md) require. The [Realism](Realism.md) runner has its own reader for scoring, and the two do not share code, because a realism check must not depend on the compiler that produced the traffic it judges.

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

## Scenario command

```bash
python3 -m capture sessions --pcap run.pcapng --payloads --out sessions.json
python3 -m capture scenario --sessions sessions.json --all --out-dir out/my_module [--synthetic]
```

The command reads a sessions document that was written with `--payloads` and writes a module directory: `manifest.yaml` and one `scenarios/<protocol>_<id>.yaml` per session. The module name is the base name of `--out-dir`, and it has to match `^[a-z][a-z0-9_]{0,62}$`.

| Option | Meaning |
|---|---|
| `--sessions` | Sessions document of schema 1.x with payloads. Required |
| `--session ID` | Converts one session. Repeat it for several. `--all` converts every session. One of the two is required |
| `--out-dir` | Module directory to write. Required |
| `--combine NAME` | Writes one scenario that runs the selected sessions in order, each as a flow that the previous flow starts with `spawn` |
| `--fill-min N` | A body of at least `N` bytes becomes `fill(N)`. The default is 256 |
| `--opaque-min N` | A run of at least `N` binary bytes (less than 85 percent printable) becomes `fill(N)`. The default is 2048 |
| `--timestamp-window S` | An epoch value is a timestamp when it lies within `S` seconds of the capture time. The default is 86400 |
| `--no-fields` | Turns field detection off. Framing stays, so every expect still has a length or delimiter, and all data stays literal |
| `--keep-timing` | Adds a `delay` for the gap between turns of one side, when the gap is at least `--min-delay-ms` (default 10) |
| `--license`, `--synthetic`, `--consent TEXT`, `--scrubbed` | Provenance of the manifest, see below |
| `--report FILE` | Writes a JSON report with the counts of detected fields per scenario |

Warnings and the counts of detected fields go to standard error. The exit code is 0 on success and 1 when the input or the output cannot be handled.

### Shape of a scenario

A TCP session becomes a client flow and a server flow, with `connect` and `accept`, and a `close` that follows the observed FIN or RST. Each turn becomes a `send` on the side that sent it and an `expect` on the other. A UDP session becomes one `send` and one `expect` per datagram, and an expect of a datagram consumes the whole datagram, so it has `bytes: N`. An `expect` on TCP uses one of three forms.

- `pattern` with a delimiter when the unit is text. The delimiters are CRLF CRLF, CRLF, and LF, and one is used only when its first occurrence ends the interval and at least 90 percent of the interval is printable.
- `bytes` with the length of a record that the framing rules below found.
- `bytes` with the whole length of the unit, as the fallback.

A unit is cut into records (head, body, frame) at the places where a length or a delimiter is known, so a flow that follows an HTTP head and then a body does not lose its place when the body differs in content.

The generator renders every template again before it writes it, with a reference implementation of the template functions. When the rendered length does not equal the observed length, it falls back to the literal bytes for that scenario and warns.

### Heuristics and their guards

Each heuristic names what it detects and what stops it from firing on ordinary data. The tests in `capture/tests/test_fields.py` and `capture/tests/test_scenario.py` have a case for the guards.

| Field | Detection | Result | False positive guard |
|---|---|---|---|
| HTTP Content-Length | A valid HTTP start line and one numeric `Content-Length` whose body lies inside the unit | A variable (`up_len`, `down_len`) that the header and `fill(vars.x)` share, when the body has at least `--fill-min` bytes. Smaller bodies stay literal | A duplicate or conflicting header, a length larger than the unit, and a start line that is not HTTP leave the message alone. Equal sizes in one direction share one variable |
| HTTP chunked body | `Transfer-Encoding: chunked` without Content-Length | Size lines stay literal, and chunk data of at least `--fill-min` bytes becomes `fill(n)` | Every size line has to be hexadecimal and every chunk and its CRLF has to lie inside the unit. At the first chunk that breaks this, the rest stays raw |
| Binary length prefix | A prefix of 1, 2, or 4 bytes at offset 0, or of 2 bytes at offset 2 or 3, big or little endian, so that the frames tile the unit exactly | Literal `hex(..)` for the prefix and `fill(N)` for the payload | The frames have to tile the unit with no remainder, the length bytes must not all be printable, and when several layouts fit, the one with the fewest frames wins. The contract has no function that writes a number as bytes, so the prefix is not a variable |
| Opaque binary | A run of at least `--opaque-min` bytes and less than 85 percent printable | `fill(N)` | Text and short runs stay literal |
| UUID | Canonical 8-4-4-4-12 text | Digits of the same shape from `rand_int` | The UUID has to occur at least twice, across a request and a response |
| Random token | A token of 7 to 63 characters from the base64 or hex alphabets | Digits of the same length and separators from `rand_int` | Needs a letter and a digit, or at least 12 digits, and at least 6 distinct characters. A run of four equal characters, a long run of letters, and a pronounceable run (6 or more letters with at least 40 percent vowels) reject it. Entropy has to reach 3.0 bits per character, or 3.3 for 16 or more characters. A token has to be echoed in both directions unless it is 16 or more characters. A token that is the only part of a request target, a whole base64 line, or a token whose 8 byte context is less than 90 percent printable is skipped |
| Echoed identifier | The same token in a request and in the response | The sender sets a variable from `rand_int`, and the receiver captures it with a split `expect` and `set: {var: "{{ last }}"}`, so both sides use one value | The token has to pass the token guards. On UDP a datagram cannot be split, so the value becomes a scenario variable |
| HTTP date | A date such as `Sun, 06 Nov 1994 08:49:37 GMT` that parses, has the right weekday, and lies within the window of the capture time | A scenario variable `http_date` that holds the observed date | The weekday has to match the day, which rejects ordinary text. There is no clock function in the contract |
| Epoch timestamp | 10 digits (seconds) or 13 digits (milliseconds) within `--timestamp-window` of the capture time | `rand_int` over a window around the observed value that keeps the digit count | The value has to be inside the window and not part of a longer run of digits |
| Cookie | A `Set-Cookie` value that a later `Cookie` header repeats | The same echo mechanism as an identifier | The value has to occur in both, in the right direction and order |

A numeric token that is long, or that has trailing zeros, would be parsed by the agent as a number when `set` stores it, and its text would change. The generator therefore puts a literal letter in front of such a token, and the byte count stays the same.

### Provenance

A capture is real traffic, so the manifest says so. The default writes `source: capture`, `consent: "not recorded"`, and `scrubbed: false`. The schema requires consent and a scrubbed capture for that source, so **the generated module fails validation until CAP-03 scrubs it and the consent is recorded**, and the command warns. `--consent TEXT --scrubbed` states both, and `--synthetic` writes `source: synthetic` for captures that a test or a lab produced. The fixture captures of the modules are synthetic, so the validation runs below use `--synthetic`.

### What the generator does not do

- A binary length prefix is a literal, because the contract has no function that writes an integer as bytes. A scenario with a prefix replays the observed length, which is correct because the generator keeps the payload at that length too.
- There is no clock function, so a date does not move with the replay time.
- Combined scenarios chain flows with `spawn` and `spawned: true`, because the contract does not say that variables pass through `after`.
- Identifier detection looks for random looking tokens. Credentials and hashes, such as an AUTH base64 string or an MD5 sum, and some user names such as `Tst-Pubrupun4007`, can be treated as identifiers. CAP-03 scrubs such values before a scenario leaves the lab.
- Encrypted traffic is opaque, so it becomes frames and `fill` only.
- Compression and content that spans turns are not undone.

## Replay test

`capture/tests/replay.py` replays generated scenarios with the real `fps-agent` in standalone mode (`run --plan --modules`) and compares the bytes of the replayed streams with the bytes of the corpus. The corpus is an HTTP exchange with a cookie, a date, an identifier and an epoch, an SMTP dialog, TLS style records, and a UDP exchange. The comparison is per port, with stream bytes for TCP and payload bytes for UDP, and the tolerance is 1 percent.

| Mode | How | Needs |
|---|---|---|
| `loopback` | Addresses in `127.71.0.0/24` and `127.72.0.0/24`, capture on `lo` | root, `tcpdump` |
| `netns` | The QA-01 namespace harness (`tests/harness/netns.sh`), pools `16.0.0.0/24` and `48.0.0.0/24`, the method of `engine/fps-agent/tests/netns_smoke.sh` | root, `tcpdump`, `ip` |

The pytest wrapper `capture/tests/test_replay_harness.py` skips with a message unless `FPS_CAP_REPLAY` is `netns` or `loopback`, `FPS_AGENT` names the agent binary, and the process is root with the tools present. By hand:

```bash
cargo build -p fps-agent
sudo env "PATH=$PATH" python3 -m capture.tests.replay --mode loopback --agent target/debug/fps-agent
```

The `cap.yml` workflow runs only `make lint-cap test-cap` on a runner without the agent, so the replay skips there. A job that runs it in the namespace harness needs these steps, in the pattern of the AGT workflow.

```yaml
  replay:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: sudo apt-get update && sudo apt-get install -y iproute2 tcpdump ethtool
      - run: python -m pip install -r capture/requirements-dev.txt
      - run: cargo build -p fps-agent
      - run: >
          sudo env "PATH=$PATH" "RUSTUP_HOME=$RUSTUP_HOME" "CARGO_HOME=$CARGO_HOME"
          FPS_CAP_REPLAY=netns FPS_AGENT="$PWD/target/debug/fps-agent"
          python -m pytest -q capture/tests/test_replay_harness.py
```

The path filters of the workflow have to include `capture/**`, `engine/**`, `tests/harness/**`, `contracts/**`, and `tools/contracts/**`. The loopback mode ran on a development machine without `ip` and the byte counts were equal on every port. The namespace mode has not run yet, so the first CI run of this job is its first test.

## Schema and versioning

[`capture/sessions.schema.json`](https://github.com/alrawi/FixingPointSystem/blob/main/capture/sessions.schema.json) is a JSON Schema (draft 2020-12) of the document, and every test validates its output against it. The document carries `schema_version` `1.0`. CAP-02 to CAP-04 may add optional fields in a minor version. A change that removes or reinterprets a field needs a new major version and an issue that names the packages that read it.

## Tests

`make lint-cap test-cap` runs `ruff format --check`, `ruff check`, `mypy --strict`, and `pytest`. `capture/tests/pcapgen.py` writes pcap and pcapng bytes, so no binary fixture is checked in. The tests cover each rule above: retransmission and overlap, out of order delivery, holes that never fill, missing handshakes and mid stream starts, SYN retransmission, resets and half closes, port reuse, sequence wraparound, IPv6, VLAN tags, snap length cuts, UDP idle splits, pcap variants, pcapng with several interfaces and resolutions, concatenated sections, and damaged files.

The tests of the scenario command run a model of the flow interpreter (`capture/tests/flowsim.py`) over generated scenarios and validate the output with `tools/contracts/validate.py`. Over the committed fixture captures of the modules (HTTP, DNS, SMTP, FTP, IMAP, and POP3, 608 sessions) every generated scenario validates with 0 errors, and the model reproduces the byte counts of each session exactly, without a deadlock.

`.github/workflows/cap.yml` runs `make lint-cap test-cap` after `pip install -r capture/requirements-dev.txt` on every change of `capture/`.

## Limits

The extractor loads the decoded packets of a capture in memory, so memory grows with the packet count and the payload size. A run over 100,000 segments of 1,400 bytes took under two seconds in a test run on the development machine. It does not reassemble IP fragments, it ignores TCP options such as SACK and window scaling, and it does not check checksums. A session that spans a UDP idle split is two sessions by design.
