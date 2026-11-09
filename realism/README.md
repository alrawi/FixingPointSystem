# Realism runner

The realism runner scores a capture against the applications that the traffic was meant to carry and writes a JSON realism report. It implements the protocol conformance criterion of the [specification](../docs/spec/specification.md#defining-realistic-traffic) with several checks. The conformance check passes when at least 99% of the sessions of every application are classified as that application, and the parser error check passes when no reference parser reports a parser error. nDPI is the classifier, and tshark and Zeek are the reference parsers. The runner calls each of them as a separate process and never links their code.

## Command

The command reads a pcap or pcapng capture and a labels file and prints the report, or writes it with `--output`.

```bash
python3 -m realism score --pcap run.pcapng --labels expected.yaml --output report.json
```

| Option | Meaning |
|---|---|
| `--pcap FILE` | Capture in pcap or pcapng format with Ethernet, Linux cooked, null, or raw IP framing |
| `--labels FILE` | Labels file in JSON, or in YAML when PyYAML is installed |
| `--output FILE` | Report path; standard output when absent |
| `--threshold R` | Conformance threshold from 0 to 1; overrides the labels file, whose default is 0.99 |
| `--ndpi-reader PATH` | ndpiReader program; defaults to `FPS_NDPI_READER` and then to `ndpiReader` on the search path |
| `--timeout S` | Seconds that ndpiReader and each parser may run, 300 by default |
| `--parsers LIST` | `auto` by default, which runs every installed parser; `none`, which leaves out the parser error check; or a comma separated list of `tshark` and `zeek`, each of which must be installed |
| `--max-duplicate-ratio R` | Largest share of duplicate payloads per application, from 0 to 1; adds the content variety check and overrides the labels file, whose default is 0.01 |
| `--scenario FILE` | Scenario file whose flow graph the session structure check compares with the capture; repeat it for several scenarios; adds the check next to the `structure` section of the labels |
| `--tshark PATH` | tshark program; defaults to `FPS_TSHARK` and then to `tshark` on the search path |
| `--zeek PATH` | Zeek program; defaults to `FPS_ZEEK`, then to `zeek` on the search path, and then to `/opt/zeek/bin/zeek` |

The exit code is 0 when every check passes, 1 when a check fails, and 2 when the capture, the labels file, ndpiReader, or a parser cannot be read or run. With `auto`, a parser that is not installed is skipped. The report lists every skipped parser with its reason in `skipped_parsers` (and by name in `missing_parsers`), the CI job summary shows it, and standard error names it. The parser error check fails when no parser ran. Gate suites and CI pass an explicit list, so a missing tool stops the run.

Commands that read captures from outside the repository run with `python3 -I`, so files next to the capture cannot shadow imports. `python3 -m pip install ./realism` installs the package for that use, as the `rlm` workflow does.

## Labels file

A labels file names the intended application of each session. [`schema/labels.schema.json`](schema/labels.schema.json) defines the format, and [`examples/http-dns.labels.yaml`](examples/http-dns.labels.yaml) shows a complete file.

```yaml
labels: fps-realism-labels
version: 1
threshold: 0.99
sessions:
  - application: http1
    match: {transport: tcp, server_port: 80}
  - application: dns
    match: {transport: udp, server_port: 53}
applications:
  http1: {ndpi: [HTTP]}
```

The runner applies four rules.

1. The first rule whose `match` fields all equal the session fields names the application. The fields are `transport`, `client_ip`, `client_port`, `server_ip`, and `server_port`.
2. A session that no rule matches takes the `default` application, or stays unlabelled and does not count.
3. A classifier result counts as the application when one of its dot separated parts equals an accepted name, so `DNS.Google` counts as `DNS`. The comparison ignores case.
4. `applications` replaces the built in accepted names, which are `HTTP` for `http1` and `DNS` for `dns`. An application without accepted names fails.

## Sessions

The runner groups TCP and UDP packets into bidirectional sessions by their 5-tuple. The sender of the first SYN without ACK is the TCP client, and the sender of the first packet is the UDP client. Frames without an IPv4 or IPv6 TCP or UDP packet, and IP fragments after the first, count as skipped frames. A session that nDPI does not report counts as `Unknown`.

We run ndpiReader with `-d`, which turns off its guess from port numbers, so only payload based detections count toward conformance.

## Report

[`schema/realism-report.schema.json`](schema/realism-report.schema.json) defines the report, which is the seam between the realism workstream and its consumers in the controller and the gate suites. The report holds the capture digest, the labels digest, and one entry per check in `checks`. The conformance entry carries `realism.conformance_ratio` from the [metric catalog](../contracts/metrics/catalog.yaml) for each application, the session counts per observed protocol, and the first 100 mismatched sessions.

A later check adds three things. It adds a definition under `$defs`, one `if` and `then` entry in `$defs.check.allOf` that selects the definition by its `check` name, and a test that validates its report entry. Every check keeps the common fields `check`, `passed`, and `metrics`, and every metric name and label comes from the catalog.

## Parser errors

The parser error check runs each reference parser on the capture and joins every finding to the session table by its 5-tuple. It reports per application the number of parser errors and malformed packets of each parser, together with the largest count of any one parser in `parser_errors` and `malformed_packets`. Taking the largest count keeps two parsers that see the same fault from counting it twice, so a capture with one malformed DNS message reports exactly one parser error. Findings in sessions without a label count in `unlabelled_errors`, findings outside every TCP or UDP session count in `unattributed_errors`, and both fail the check. The entry also lists the first 100 findings in `errors`.

| Parser | What counts as one parser error | Command |
|---|---|---|
| tshark | One frame with expert information of error severity, which includes every malformed packet | `tshark -r FILE -n -Y '_ws.expert.severity >= 0x00800000' -T fields ...` |
| Zeek | The `weird.log` entries of one connection at one packet time, except the state weirds in `ZEEK_STATE_WEIRDS` | `zeek -C -r FILE LogAscii::use_json=T` in a temporary directory |

Zeek runs with `-C`, so checksums that the capturing host left to the network card do not count, and tshark leaves checksum validation off by default. The state weirds, such as `dns_unmatched_reply` and `possible_split_routing`, describe session behavior rather than a message that failed to parse. The check leaves `metrics` empty until the metric catalog defines a parser error metric, which issue #89 tracks.

`realism/parsers.py` defines the `ParserRunner` protocol, which takes a capture and returns its findings. `TsharkRunner` and `ZeekRunner` run the real programs, and `StubParser` returns fixed findings for the tests.

## Encryption

The encryption check decrypts a sample of TLS sessions with the exported keys and compares the JA4 fingerprint of each ClientHello with the one that the client profile of the session expects. It runs when the labels file has a `tls` section, and it needs tshark 4.2 or newer.

```yaml
tls:
  sample: 100            # most sessions to sample, default 100
  min_decrypted: 1.0     # share that must decrypt, default 1
  profiles:
    chrome:
      ja4: t13d1516h2_8daaf6152771_e5627efa2ab1   # or a list of fingerprints
sessions:
  - application: https
    tls_profile: chrome
    match: {transport: tcp, server_port: 443}
```

Only sessions whose rule names a `tls_profile` take part. The check takes at most `sample` of them, spread evenly over the order of their first packet, and runs tshark once with the keys that a pcapng capture embeds in its decryption secrets blocks and with the key log that `--keylog FILE` names. A session counts as decrypted when tshark reads the Finished message of both sides, which TLS 1.2 and 1.3 encrypt. `realism/ja4.py` reads the ClientHello bytes of the session and computes JA4 from the FoxIO specification, and the entry also carries the fingerprint that tshark computes. A session fails when its JA4 is not one that its profile lists, or when tshark and the runner compute different values. The entry names every reason in `failures`, with the parts of the fingerprint that differ, for example `part a (version, SNI flag, cipher and extension counts, ALPN) is t13d1517h2, expected t13d1516h2`. The check also fails when no session carries a `tls_profile`, when tshark is missing, and when fewer than `min_decrypted` of the sample decrypt.

## Content variety

The content variety check reassembles the application payloads of every session, counts exact duplicates, and describes the size distribution. It runs when the labels file has a `content` section, which may be empty, or when `--max-duplicate-ratio` is given.

```yaml
content:
  max_duplicate_ratio: 0.01   # default 0.01
  ignore_headers: [x-nonce]   # on top of the built in list
  sizes:                      # optional expected distribution per application
    http1: {direction: server, median: 1200, cv: 0.8, tolerance: 0.25}
```

A payload unit is one HTTP/1 message (for an application whose name starts with `http`, or when the client stream starts with an HTTP method), one UDP datagram, or one whole TCP direction for any other protocol. The runner joins the TCP segments of each direction in sequence order, and a hole cuts the stream (`incomplete_directions`). The digest of a unit is the SHA-256 of its normalized form.

| Protocol | What the digest covers |
|---|---|
| HTTP/1 | The start line, the headers in wire order with lower case names and trimmed values, and the body after the chunked framing is removed. The headers `date`, `expires`, `last-modified`, `age`, `etag`, `if-modified-since`, `if-none-match`, `set-cookie`, `cookie`, `x-request-id`, `request-id`, `x-trace-id`, `x-amzn-trace-id`, `traceparent`, `tracestate`, and `server-timing` are dropped, and `ignore_headers` adds more. `Content-Length` stays. A message without a body still counts. |
| DNS (application `dns`) | The datagram without its 2 byte transaction identifier |
| Other | The bytes as they are |

The duplicate ratio of a group is `(units - distinct digests) / units`. A group is the units of one application in one direction, so a request is never a copy of a response, and an application is `unlabelled` when no rule names it. The check fails when the ratio of any application is above `max_duplicate_ratio`, and the entry names the most repeated payload with its size, copies, digest, and sessions. The entry reports the size of each unit in bytes on the stream, as count, minimum, maximum, mean, population standard deviation, coefficient of variation, and the quantiles p5, p25, p50, p75, p95, and p99, per application and per direction. With `sizes` in the labels, the observed median, and the observed coefficient of variation when `cv` is given, must lie within `tolerance` (default 0.25) of the expected values for the payloads of `direction` (`client`, `server`, or `both`), or the check fails. Without `sizes` the dispersion is reported and nothing fails on it.

Sessions with a `tls_profile`, and sessions whose stream starts with a TLS record, are encrypted. The runner skips them and counts them in `skipped_encrypted_sessions`, because the encryption check decrypts only to find Finished messages and exposes no plaintext. An HTTP message that the end of the stream cuts short is dropped (`truncated_units`). The check fails when no payload at all could be analysed.

## Session structure

A scenario instance opens several connections, and a generator that drops one, repeats one, or runs them in the wrong order produces traffic that no real client makes. The session structure check reads the flow graph of a scenario file, rebuilds the connections of each instance from the capture, and compares both. It runs when the labels file has a `structure` section or the command has `--scenario`.

```yaml
structure:
  scenarios:
    - file: ../scenarios/web_browse.yaml   # relative to the labels file
      vars: {server_port: 9090}            # optional, for a port that the run changed
      match: {client_ip: 10.0.0.10}        # optional, limits the sessions it may claim
      expected_instances: 40               # optional
  max_gap_s: 30                            # optional, a silent client starts a new instance
  tolerance_s: 0.002                       # optional slack for the ordering rules, default 0
```

[`examples/web_browse.labels.yaml`](examples/web_browse.labels.yaml) and [`examples/web_browse.scenario.yaml`](examples/web_browse.scenario.yaml) show a complete pair. This is the `web_browse` scenario of the specification, a DNS lookup flow followed by a page flow that runs `after` it.

**Graph.** [`scenario_graph.py`](scenario_graph.py) reads the scenario with PyYAML and no template engine. A flow is one connection. A flow that is not spawned occurs once per instance. A spawned flow occurs as often as its parent spawns it, where a `spawn` in a `loop` repeats `count` times (a literal or `{{ vars.name }}`), a `spawn` in a `branch` may not happen, and a count that the file does not fix allows any number. The two sides of the parent both run `spawn` for the same child, and the check counts the child once. The edges are the `after` lists and the `spawn` actions.

**Which connection is which flow.** A session is a flow when the transport equals `l4` and the server port equals the port of the flow, which the `accept` of the server program or the `connect` of the client program gives as a literal or a `{{ vars.name }}` reference. A flow without such a port, such as an FTP data connection that a reply announces, matches any session of its transport on a port that no other flow of the scenario fixes, and flows that cannot be told apart take the connections in the order in which the scenario spawns them.

**Which connections form an instance.** The runner groups the sessions of a scenario by client address and walks them in order of their first packet. A session joins the open instance of its client while that instance has room for its flow, meaning fewer connections of the flow than the graph allows. Otherwise it opens a new instance, and so does a client that was silent for longer than `max_gap_s`. A session that matches no flow, or that no scenario entry may claim, counts in `sessions_unassigned` and is not judged. A session joins an instance by its flow and not by its position, so a page fetch that starts before its lookup still joins the instance of that lookup, and the ordering rule then names the broken edge.

The grouping has limits.

- One client address that runs several instances at the same time cannot be split. The extra connections open an instance of their own that lacks its other flows.
- Clients behind one address look like one client.
- A missing or repeated connection shifts the later connections of that client into other instances, so one fault can fail the next instance too. The failure of such an instance says that it opened because the instance before had no room.
- Flows of one transport without a fixed port are interchangeable. The check counts them and checks them against the `spawn` edges, and it cannot tell which of them a connection is.
- Scenarios that share a transport, a port, and a client address cannot be told apart in one capture, so give each its own `match`, or record one scenario per capture. The `ftp.pcapng` fixture mixes five scenarios on one address and one control port, so no single scenario file describes it.

**Rules.** Per instance and flow, the number of connections must lie within what the graph allows. Per `after` edge, every connection of the later flow starts no earlier than every connection of the earlier flow completed. A session completes at its last packet with payload, FIN, or RST, and the closing ACK does not count, because the engine starts the next flow when the close action returns. Per `spawn` edge, every connection of the child starts between the first and the last packet of some connection of the parent. `tolerance_s` loosens both comparisons. The check also fails when a scenario matches no session, and when `expected_instances` disagrees with the number of instances.

A failure names the scenario, the instance with its client address and first packet, and the broken edge with both sessions and their times relative to the instance, for example `scenario web_browse instance 1 (client 10.0.0.10, first packet 1700000000.001000): edge after lookup -> page violated: page (session 1, tcp 10.0.0.10:40001 -> 10.0.1.80:8080) starts at +0.000000 s, before lookup (session 2, udp 10.0.0.10:40000 -> 10.0.1.53:53) completes at +0.030999 s`. The entry lists the flows with their allowed and observed connection counts, the edges, the first 50 failed instances with their connections, and the first 50 failures (`failures_total` counts all). It leaves `metrics` empty until the catalog has a metric for it.

## Classifiers

`realism/classifier.py` defines the `Classifier` protocol, which takes a capture and its sessions and returns a protocol name per session. `NdpiClassifier` runs ndpiReader and parses its CSV output by column name, so it reads nDPI 4.2 and later layouts. `StubClassifier` labels sessions with a function and serves the tests.

## Tests

The tests build every capture in code with [`tests/pcapgen.py`](tests/pcapgen.py), so the repository holds no captured traffic. They replace ndpiReader, tshark, and Zeek with stubs or with fake programs that print fixed output. Three optional modules run the real tools and skip each tool that is not installed. `tests/test_ndpi_reader.py` runs ndpiReader, `tests/test_parser_tools.py` runs tshark and Zeek on a capture with one malformed DNS message and on clean captures, and `tests/test_encryption.py` runs tshark on TLS captures that `tests/tlsgen.py` builds from real handshakes of the Python `ssl` module (with `openssl` for the certificate). `tests/test_content.py` builds every content capture in code, with repeated and with varied bodies, segmented and reordered TCP, and encrypted sessions. `tests/test_structure.py` builds web_browse, ftp, and dns captures in code and reads the first-party scenario files under `modules/`. `tests/test_ja4.py` checks the JA4 code against the worked example of the specification. The `rlm` workflow installs `libndpi-bin` and `tshark` from Ubuntu and `zeek-core` from the package repository of the Zeek project, so CI runs every optional test.

```bash
python3 -m pip install -r realism/requirements-dev.txt
make lint-rlm test-rlm
```

## Module realism job

`python3 -m realism modules` prints the native modules that a change set selects for the module realism job of the `realism` workflow, as a GitHub Actions matrix or with `--format names` one name per line. `--changed FILE` reads the changed paths, one per line, and `-` reads them from standard input. Without `--changed` every module is selected. [`ci/module-realism.sh`](ci/module-realism.sh) records and scores one module, [`ci/install-tools.sh`](ci/install-tools.sh) installs the tools on a runner, and [`ci/negative_control.py`](ci/negative_control.py) breaks the http1 request line for the negative control. The wiki page Realism describes the job.
