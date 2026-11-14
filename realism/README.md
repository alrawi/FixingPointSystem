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
| `--ks-alpha A` | Significance level of the KS tests, above 0 and below 1; adds the temporal distribution check and overrides the labels file, whose default is 0.01 |
| `--ks-min-samples N` | Fewest observations that a KS comparison needs, 2 or more; adds the temporal distribution check and overrides the labels file, whose default is 30 |
| `--test-config FILE` | Test configuration whose `profile.mix` weights and `profile.basis` the mix accuracy check compares with the achieved mix; adds the check next to the `mix` section of the labels |
| `--run-report FILE` | File with the achieved sessions and bytes per scenario (see [Mix accuracy](#mix-accuracy)); adds the check |
| `--mix-tolerance P` | Largest difference between an achieved and a configured share in percentage points, 0 to 100, default 2; adds the check and overrides the labels |
| `--mix-basis B` | `sessions`, `bytes`, or `both`; default `profile.basis` of the test configuration; adds the check and overrides the labels |
| `--os-profile NAME` | Operating system profile of every host (`windows11`, `linux6`, or `macos14`); adds the transport behavior check next to the `transport` section of the labels and overrides its `os_profile` |
| `--os-profiles-dir DIR` | Directory of the profile files; defaults to the profiles of the kernel backend; adds the check |
| `--transport-strict` | Also fail on the TCP option order and the SYN window; adds the check |
| `--client-hops N`, `--server-hops N` | Routers between the clients, or the servers, and the capture point, 0 to 254, default 0; they lower the TTL that a SYN or a SYN-ACK shows; add the check |
| `--host-identity` | Adds the host identity check next to the `hosts` section of the labels |
| `--host-count N` | Number of hosts that the capture must hold; adds the host identity check and overrides `hosts.count` of the labels file |
| `--attach-run ID` | After scoring, deliver the report to the controller with `PUT /v1/runs/ID/realism` (see [Delivering the report](#delivering-the-report-to-the-controller)) |
| `--url URL`, `--token-file FILE`, `--ca-file FILE`, `--insecure-http` | The controller and the run token for `--attach-run`, which read `FPS_URL` (default `https://localhost:8443`), `FPS_TOKEN_FILE`, `FPS_CA_FILE`, and `FPS_INSECURE_HTTP` when absent; each needs `--attach-run` |
| `--tshark PATH` | tshark program; defaults to `FPS_TSHARK` and then to `tshark` on the search path |
| `--zeek PATH` | Zeek program; defaults to `FPS_ZEEK`, then to `zeek` on the search path, and then to `/opt/zeek/bin/zeek` |

The exit code is 0 when every check passes, 1 when a check fails, and 2 when the capture, the labels file, ndpiReader, or a parser cannot be read or run, or when `--attach-run` was given and the controller did not accept the report. With `auto`, a parser that is not installed is skipped. The report lists every skipped parser with its reason in `skipped_parsers` (and by name in `missing_parsers`), the CI job summary shows it, and standard error names it. The parser error check fails when no parser ran. Gate suites and CI pass an explicit list, so a missing tool stops the run.

Commands that read captures from outside the repository run with `python3 -I`, so files next to the capture cannot shadow imports. `python3 -m pip install ./realism` installs the package for that use, as the `rlm` workflow does.

## Delivering the report to the controller

The controller measures no realism itself. `--attach-run ID` sends the report that `score` just wrote (the same bytes as `--output`) to `PUT /v1/runs/ID/realism` (`attachRealismReport`, CTL-41), so that `realism` of the run report holds its results.

```bash
export FPS_URL=https://controller.example:8443 FPS_TOKEN_FILE=$HOME/.config/fps/token FPS_CA_FILE=ca.pem
python3 -m realism score --pcap run.pcapng --labels expected.yaml --output report.json --attach-run run-7
```

The run token needs the scope `run`. It comes from `--token-file`, then `FPS_TOKEN_FILE`, then `FPS_TOKEN`, as for the `fps` client. There is no `--token` option, because a command line shows in the process list, and `score` accepts no abbreviated option names, so `--token SECRET` is an error and never reads `--token-file`. A token file that other users can read gets a warning. The runner sends the token as `Authorization: Bearer`, replaces it with `[redacted]` in any text that the server sends back, never follows a redirect, and refuses `http` to a host other than loopback unless `--insecure-http` is set. It reads the token and checks the URL before it scores, so a missing token stops the command at once.

A report is delivered whether its checks pass or fail, since a failing report is the one that an operator wants to see. The controller answers in this order (see [Controller](../docs/wiki/Controller.md#realism-reports-attached-to-a-run)), and the runner prints the first failing step on standard error and exits with 2. The report file stays on disk.

| Answer | Output of the runner |
|---|---|
| 200 | `attached to run ID: the controller holds N results, P passed and F failed`, plus `(replaced the earlier report)` when the run held one. The exit code is that of the checks, 0 or 1 |
| 409 | The problem detail, which names the state of a run that has not ended |
| 422 | The problem detail and one line `POINTER: message` for each finding, so a report that breaks the schema names the fields |
| 401, 403, 404, 413, 415, other | `HTTP status` with the title and detail of the problem, or the start of a body that is no problem document |
| No answer | `could not reach the controller` with the reason |

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

A later check adds three things. It adds a definition under `$defs`, one `if` and `then` entry in `$defs.check.allOf` that selects the definition by its `check` name, and a test that validates its report entry. Every check keeps the common fields `check`, `passed`, and `metrics`, and every metric name and label comes from the catalog. Version 0.10.0 added `metrics_basis` and the series to the `mix_accuracy` entry and the `rest_report` form of its run report, version 0.11.0 added the `messages` entry, version 0.12.0 added the metric series of every check except `messages` (see below), version 0.13.0 added the `host_identity` entry, version 0.14.0 added the series of the `messages` check, version 0.15.0 added the series of the `host_identity` check, version 0.16.0 added the `packet_gap` property of the `temporal` entry (RLM-15), version 0.19.0 added the expected findings of the `parser_errors` entry (RLM-20, see below), and version 0.20.0 added `path` and `min_samples` to the comparisons of the `temporal` entry and its `rungs` list (RLM-22, see below), and version 0.21.0 added `session`, `transport`, `client`, and `server` to the entries of `missing_findings` (RLM-23, see below). The labels file stays at `version: 1`, because every section is optional and a new section adds no field to the others.

## Metric series of each check

Report format 0.17.0 and later carry the series of metric catalog 0.8.0 (`contracts/changes/CON-23.md`, `CON-25.md`, `CON-27.md`, and `CON-28.md`). Every check of the kinds `conformance`, `parser_errors`, `encryption`, `content`, `session_structure`, `temporal`, `mix_accuracy`, `transport_behavior`, `messages`, and `host_identity` has exactly one `realism.check_failures` series with the label `check` set to its kind, and the schema requires it. Its value is the number of reasons for the failure and 0 exactly when the check passed, and a failed check never shows 0, even when it names no reason. The other series are:

| Check | Series | `realism.check_failures` |
|---|---|---|
| `conformance` | `realism.conformance_ratio` per scored application | Applications that failed, plus 1 when none was scored |
| `temporal` | `realism.ks_statistic` and `realism.ks_p_value` per comparison with a catalog label: `inter_arrival`, `request_bytes`, `response_bytes`, and for packet gaps `request_gap` (client streams), `response_gap` (server streams), and `packet_gap` (both directions), but not for a paced comparison, whose statistic is no KS statistic, nor for a comparison with a `path`, nor for the `rungs` | `len(failures)` |
| `parser_errors` | `realism.parser_errors` per application and parser, and per parser with `application` `all` | `errors_total`, plus one for each missing expected finding, plus 1 when no parser ran |
| `encryption` | `realism.decrypted_ratio` and `realism.ja4_match_ratio` when sessions were sampled | `len(failures)` |
| `content` | `realism.duplicate_ratio` per application | `len(failures)` |
| `session_structure` | `realism.structure_failures` per scenario (scenarios of one name add up) | `failures_total` |
| `mix_accuracy` | `mix.share_pct`, `mix.abs_error_pp`, and `mix.max_abs_error_pp` with `source` `classifier` | `failures_total` |
| `transport_behavior` | `realism.transport_deviations` per enforced field and side | `failures_total` |
| `messages` | none | `failures_total` |
| `host_identity` | none | `failures_total` |

The `messages` check carries only the failures series since CON-25 (report format 0.14.0, catalog 0.6.0), and the `host_identity` check since CON-27 (report format 0.15.0, catalog 0.7.0). Both values are `failures_total`. A report holds at most one check of each kind (the runner writes one entry per kind), and the controller refuses a document with two checks of one kind, because the second repeats the failures series of the first. `realism/series.py` builds the failures series, and `realism/tests/test_check_metrics.py` pins the literal series of every kind, passing and failing, and validates the examples of `contracts/examples/realism/check-metrics.yaml` against the schema.

## Parser errors

The parser error check runs each reference parser on the capture and joins every finding to the session table by its 5-tuple. It reports per application the number of parser errors and malformed packets of each parser, together with the largest count of any one parser in `parser_errors` and `malformed_packets`. Taking the largest count keeps two parsers that see the same fault from counting it twice, so a capture with one malformed DNS message reports exactly one parser error. Findings in sessions without a label count in `unlabelled_errors`, findings outside every TCP or UDP session count in `unattributed_errors`, and both fail the check. The entry also lists the first 100 findings in `errors`.

**Port collision rule (QA-11, RLM-19).** The kernel picks the client port of a session at random, and tshark binds about 900 TCP and UDP ports to a dissector (`tshark -G decodes` lists them), among them UDP 41170 (Manolito) and TCP 44322 (Performance Co-Pilot proxy). When a session uses such a port as its client port, tshark decodes the payload with that dissector and reports every packet that the dissector rejects as malformed, so a well formed session gets findings by chance (QA-11 saw 31 in one Gate 2 run, and a module job once failed on 3). A tshark finding is a port collision, and does not count, when the client port of its session is bound to a dissector for the transport of the session and the server port is not bound to the same dissector, so that the intended protocol would have taken the session. The check lists each such finding under `not_counted` (the first 100, with the same fields as an error plus `rule`, which is `port_collision`, and `dissector`), counts all of them in `not_counted_total`, and keeps them out of `errors_total`, `parser_errors`, `malformed_packets`, the metric series, and the pass verdict. `port_collision_ports` is the number of ports in the table that tshark printed, and it is 0 when no table was available, in which case every finding counts. The rule never applies to Zeek, whose weirds do not depend on the port, and never to a finding outside every session, which has no client port. It hides a real fault only when that fault lies in a session whose client port collides, and the finding stays in `not_counted`. The runner reads the table with `TsharkRunner.port_bindings()` (once per run), and the job summary of a module names the number of findings that did not count. Report format 0.18.0 added the three fields.

**Expected findings (RLM-20).** A module whose traffic is malformed on purpose (the strike module, a fuzzer) names the findings that its sessions must produce. A session rule of the labels takes `expect_findings`, a mapping from a parser to a list of items.

```yaml
sessions:
  - application: http1
    match: {transport: tcp, server_port: 8080}
    expect_findings:
      zeek: [bad_HTTP_request]
      tshark: ["http: Illegal characters"]
```

A Zeek item is the name of a weird, which `weird.log` prints in its `name` column. A tshark item is the protocol of the frame that carries the expert item, which is the last name in `frame.protocols` (`http`, `dns`, `tcp`), optionally followed by a colon, a space, and a part of the expert message that the item must contain (`http: Illegal characters`). tshark reports one finding per frame and keeps the message of the first expert item of error severity, so the message part is matched against that one. The first rule that matches a session applies, as for its application, and the items of a rule cover only the sessions that the rule names first.

A finding that a rule of its session declares is an expected finding. It stays out of `errors_total`, `parser_errors`, `malformed_packets`, and the metric series, and the entry lists it under `expected_findings` (the first 100, each an error entry with `session_rule`, the index of the rule in `sessions`) and counts it in `expected_total`. Expected findings come before the port collision rule, so a malformed session whose client port collides still shows its declared items. A Zeek finding names every weird of one connection at one packet time, and it is expected only when every one of those weirds is declared. The check fails in two ways, and the report lists both.

- **Missing.** A declared item that no session of its rule produced, for a parser that ran, is listed under `missing_findings` (`session_rule`, `application`, `parser`, `item`) and adds 1 to `realism.check_failures` for each item. Without `expect_every_session`, an item is satisfied by one finding in any session of its rule, so a rule over many sessions does not say that each one is malformed. With it, the entry also names the session (see below).
- **Undeclared.** A finding that no rule of its session declares counts as before and is in `errors`, whether its session has other declarations or not.

A declared item of a parser that did not run (Zeek is optional) cannot be checked. It is listed under `unchecked_expectations` and does not fail the check, so score with `--parsers tshark,zeek` to require both. The job summary of a module names the number of expected findings, and the number that are missing. Report format 0.19.0 added the four fields.

**Every session of a rule (RLM-23).** A rule that sets `expect_every_session: true` holds each of its sessions to its `expect_findings`, so a scenario that is malformed in only some of its sessions fails. The setting needs `expect_findings` in the same rule, and it is optional and false by default, so a rule without it keeps the behaviour above.

```yaml
sessions:
  - application: http1
    match: {transport: tcp, server_port: 8080}
    expect_findings:
      tshark: ["http: Illegal characters"]
    expect_every_session: true
```

Under the setting, every session that the rule matches first must show every declared item of a parser that ran. A session that lacks an item adds one entry to `missing_findings`, with the usual `session_rule`, `application`, `parser`, and `item`, and with `session` (the session id of the capture), `transport`, `client`, and `server`, and each entry adds 1 to `realism.check_failures`. A finding of one session never meets the item of another. The other parts of the check do not change: a finding that the rule declares is expected, a finding that no rule declares counts, and an item of a parser that did not run is unchecked. A rule with the setting that matches no session lists each item once, without a session, as a rule without it does. The rule selects sessions by transport, addresses, and ports only, so give each scenario its own `server_port` and its own rule, as above, and keep a well formed scenario out of a rule that sets `expect_every_session`. Report format 0.21.0 added the four session fields to the entries of `missing_findings`.

| Parser | What counts as one parser error | Command |
|---|---|---|
| tshark | One frame with expert information of error severity, which includes every malformed packet | `tshark -r FILE -n -Y '_ws.expert.severity >= 0x00800000' -T fields ...` |
| Zeek | The `weird.log` entries of one connection at one packet time, except the state weirds in `ZEEK_STATE_WEIRDS` | `zeek -C -r FILE LogAscii::use_json=T` in a temporary directory |

Zeek runs with `-C`, so checksums that the capturing host left to the network card do not count, and tshark leaves checksum validation off by default. The state weirds, such as `dns_unmatched_reply` and `possible_split_routing`, describe session behavior rather than a message that failed to parse. `premature_connection_reuse` belongs to them: Zeek raises it for a SYN that reuses the 5-tuple of a connection it still tracks, which happens by chance when an agent picks a source port again, and the runner's session of that 5-tuple then holds more than one TCP connection (QA-09). The session structure check sees the connections. The entry carries `realism.parser_errors` for each application and parser, and for the application `all` of each parser.

`realism/parsers.py` defines the `ParserRunner` protocol, which takes a capture and returns its findings. `TsharkRunner` and `ZeekRunner` run the real programs, and `StubParser` returns fixed findings for the tests. A runner also answers `port_bindings()`, the table for the port collision rule, which is empty for every runner but tshark.

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

A failure names the scenario, the instance with its client address and first packet, and the broken edge with both sessions and their times relative to the instance, for example `scenario web_browse instance 1 (client 10.0.0.10, first packet 1700000000.001000): edge after lookup -> page violated: page (session 1, tcp 10.0.0.10:40001 -> 10.0.1.80:8080) starts at +0.000000 s, before lookup (session 2, udp 10.0.0.10:40000 -> 10.0.1.53:53) completes at +0.030999 s`. The entry lists the flows with their allowed and observed connection counts, the edges, the first 50 failed instances with their connections, and the first 50 failures (`failures_total` counts all). The entry carries `realism.structure_failures` for each scenario.

## Temporal distribution

The temporal check compares the inter arrival times of sessions, the payload sizes, and the gaps between the packets of one session with target distributions of the labels through the KS test. It runs when the labels file has a `temporal` section, or when `--ks-alpha` or `--ks-min-samples` is given.

```yaml
temporal:
  alpha: 0.01          # default; a comparison fails when its p value is below it
  min_samples: 30      # default; fewer observations report insufficient_data
  arrivals:            # gaps in seconds between the first packets of consecutive sessions
    overall: {dist: exponential, mean_ms: 142.9}     # every session of the capture
    applications:
      http1: {dist: exponential, mean_ms: 200}       # Poisson, 5 sessions per second
      dns: {dist: uniform, min_ms: 100, max_ms: 900}
  sizes:               # bytes of each payload unit, as the content check defines it
    applications:
      http1:
        - {direction: client, dist: lognormal, mu: 5.5, sigma: 0.4}
        - {direction: server, dist: pareto, alpha: 1.8, scale: 500}
  packets:             # gaps in seconds between the packets of one direction of a session
    applications:
      rtp: {dist: constant, ms: [20, 30], tolerance_ms: 5, within: 0.9}   # paced media
      hls_vod:                                                  # the segment requests of a keepalive connection
        {direction: client, path: '/v\d+/seg\d+\.ts$', dist: constant, ms: 400, tolerance_ms: 40, mean_tolerance_ms: 20, min_samples: 3}
  rungs:               # bytes per rung of an adaptive bit rate playback
    applications:
      hls_vod: {path: '/v(\d+)/seg\d+\.ts$', duration_ms: 400, bits_per_second: [160000, 320000, 640000], walk: up, streams: 3, min_samples: 4}
```

A target is `constant`, `exponential`, `uniform`, `lognormal`, `pareto`, or `empirical`. Arrivals use the parameter names of a scenario `delay` (`mean_ms`, `min_ms`, `max_ms`, `mu` with `sigma` and `unit`, `alpha` with `scale_ms`, and `ms` for a constant, which may stand without `dist`), and sizes use bytes (`value`, `mean`, `min`, `max`, `mu`, `sigma`, `alpha`, `scale`). An `empirical` target gives `sample` (a list of values), `sample_file` (numbers separated by white space or commas), or `reference` (`{pcap, labels}`, a capture whose sessions give the sample), all relative to the labels file, and runs the two sample test. An application can have one size target per direction, written as a list.

The `packets` section checks the pacing inside a session, which `arrivals` cannot see. It reads the packets with payload of each direction of each session as a stream (a gap never spans two directions, and a TCP acknowledgment without payload does not count) and compares the gaps with the target of the application, or of `overall`. It takes the parameter names of `arrivals`, and an application can have one target per direction (`direction: client`, `server`, or `both`, the default), written as a list. Every distribution but the constant runs the KS test on the gaps of all its streams together. The constant is the packet time of paced media and is judged differently, because a real sender is a few milliseconds late and a loaded host sends a burst after a stall:

* `ms` is one packet time or a list of them, because the calls of one application can differ (the sip_rtp fixture has 20 ms and 30 ms calls). Each stream is compared with the listed time that is nearest to its median gap.
* `tolerance_ms` is required and says how far a gap may be from the packet time.
* `within` (default 0.9) is the share of the gaps of each stream that must be within the tolerance.

Each stream with at least `min_samples` gaps is judged alone, so one jittered call cannot hide among good ones. A shorter stream counts in the sample size but is not judged alone, and the comparison reports `insufficient_data` when no stream is long enough. The comparison fails when a judged stream fails, and the reason names the application, the number of failed streams, the worst stream with its session, ports, and side, the share of late gaps, and the observed range, for example `packet gaps of application rtp: 2 of 4 streams are not paced at 20 ms, because more than 10.0% of their gaps are further than 5 ms from the packet time; the worst is the udp stream 10.201.0.2:40002 to 10.202.0.2:20002 (session 2, client side), where 39.4% of 99 gaps are further than 5 ms from 20 ms (gaps from 6.14309 ms to 33.8569 ms, median 20 ms)`. The entry has the statistic (the largest share of late gaps of any judged stream), the p value (1 when every judged stream passed, 0 otherwise), and `streams` with the numbers judged, failed, and too short. The test bounds late and early packets but not a slow drift inside the tolerance, which the mean in `observed` shows. The metric catalog has no property for packet gaps, so they add no `realism.ks_*` series to the entry.

**Requests of one kind (RLM-22).** A keepalive connection of an HTTP application alternates requests of several kinds, such as the playlist and the segments of HLS, so the gaps of its packets mix cadences. A target of the client direction can select the requests of one kind with `path`, a regular expression that is searched in the request target (the path and the query of the request line). The stream of a session then holds only the matching requests, timed by the first packet of the request, and a gap is the time between two of them. The paced constant, the KS distributions, and the empirical targets all take it. The reader follows HTTP/1.1 `GET` requests with a `Content-Length` response, and a session of the application that it cannot follow fails the target with the reason and the session. A path target belongs under `applications` (the sessions of the application are the ones that speak HTTP), needs `direction: client`, and two targets of one application and direction differ by their path. `join_ms` makes the matching requests of all sessions of the application one stream while the gap to the next one stays below it, which is a playback whose segments each use a connection (the `vod_segments` scenario of the hls module). A comparison with a path adds no `realism.ks_*` series, because a second path of the same application would give a second series with the same labels.

**Samples per target and the mean (RLM-22).** Every target takes `min_samples`, an integer of 2 or more that replaces the value of the section and of `--ks-min-samples` for this target, because a playback has far fewer gaps than a stream of media (8 to 32 gaps against 30). A paced constant also takes `mean_tolerance_ms`: the mean gap of each judged stream must lie within it of the packet time, so a cadence that drifts inside `tolerance_ms` still fails (400 ms with a tolerance of 40 ms and a mean tolerance of 20 ms is the 5% of a segment duration that `hls_check.py` allowed). A stream that fails only in its mean is named in the reason.

**Bytes per rung (RLM-22).** The `rungs` section reads the response sizes of the requests that a path selects, for an adaptive bit rate playback whose request names the rung, such as `/v1/seg4.ts`. Each application gives `path` (its first group is the rung number), `duration_ms` (the media time of one response), and `bits_per_second` (the rate of each rung). Every stream of requests (one session, or with `join_ms` the sessions of a playback that follow each other) with at least `min_samples` requests must have

* the body of every response within `body_tolerance_bytes` (default 188, one MPEG transport stream packet) of `rate * duration / 8` of its rung;
* the share of every rung in the bytes of the stream within `share_tolerance` (default 0.05, relative) of the share that the rates of the requested rungs give, where the bytes are the octets of the response head and body;
* the bit rate of the stream, those octets over the media time of its requests, within `rate_tolerance` (default 0.05, relative) of the mean rate of the requested rungs;
* with `walk: up`, rungs that rise and visit the whole ladder.

`streams` is the number of judged streams that the capture must hold, a number or a range, so a playback that is missing or too short fails. A stream with fewer requests than `min_samples` is counted under `streams.too_short` and not judged, and an application without a long stream reports `insufficient_data`. The report gives each entry under `rungs`: the judged streams with their requests, bit rate, configured bit rate, and rung shares (those that failed first, at most 100), and the reason in words for a failure, for example `bytes per rung of application hls_vod: 1 of 3 streams carry the wrong bytes; the first is the tcp session 10.201.0.2:40001 to 10.202.0.2:8090 (session 2): /v2/seg6.ts: body of 25600 octets against 32000`. The metric catalog has no series for the bytes, so a failure shows in `failures` and in `realism.check_failures`. The packet gap target and the rung target together are what `modules/hls/fixtures/hls_check.py` of MOD-15 checks: the cadence of the segment requests, the bytes of each segment, the byte share of each rung, and the bit rate of each playback.

Each comparison reports its status, which is `pass` when the p value is at least `alpha`, `fail` below it, and `insufficient_data` when a sample has fewer than `min_samples` observations. It also reports the statistic, the p value, the method (`exact` or `asymptotic`), the sample size, the observed minimum, median, mean, and maximum, and a reason in words for anything but a pass. A comparison that is not a pass fails the check, and so does a section without targets. Constants are exact: the observations must lie within `tolerance` of the value. The p value is exact for up to 100 observations (one sample) or 10000 pairs (two samples), and asymptotic above that. [`ks.py`](ks.py) implements the tests with the standard library only. The KS statistic and p value of each application go into the report as `realism.ks_statistic` and `realism.ks_p_value`, with the properties `inter_arrival`, `request_bytes`, and `response_bytes`.

## Mix accuracy

The mix accuracy check compares the traffic mix that a run achieved with the weights of its test configuration, in two independent ways: from the run metrics and from nDPI on the capture. It runs when the labels file has a `mix` section or when the command has `--test-config`, `--run-report`, `--mix-tolerance`, or `--mix-basis`, so a labels file without either behaves as before. [`examples/mix.labels.yaml`](examples/mix.labels.yaml) shows the section.

```yaml
mix:
  scenarios:                     # explicit: each scenario of the mix and the application it stands for
    http1/web_get: http1         # the nDPI names are those of the conformance check (applications section)
    http1/web_browse: http1      # scenarios of one application form one row
    ftp/get: {application: ftp, ndpi: [FTP_CONTROL]}   # or list nDPI names for this entry
  test_config: test.yaml         # or --test-config; or weights: {scenario: weight} with basis
  run_report: run.json           # or --run-report
  basis: both                    # sessions, bytes, or both; default profile.basis of the test configuration
  tolerance_points: 2            # default 2
```

**Inputs.** The weights come from `profile.mix` of the test configuration (a scenario listed twice counts with the sum of its weights). The achieved counts per scenario come from the run report file, in JSON or YAML, in one of four forms:

* the metric series of `GET /v1/runs/{id}/metrics?view=scenario`, from which the check takes the last sample of every key `<application>/<scenario>` and reads `sessions.attempted`, `bytes.tx`, and `bytes.rx` (fetch it with the default `side=client`, which counts each byte once),
* a bundle object with that series under `metrics`, next to the `run` of `GET /v1/runs/{id}/report`, and
* a table `{"scenarios": [{"scenario": "http1/web_get", "sessions": 600, "bytes": 123456}]}`, which is the file form of the controller report (CTL-38), and
* the `RunReport` of `GET /v1/runs/{id}/report` itself. Its `scenarios` list (API 0.9.0, CON-21) gives the application and the scenario apart, so the check joins them as `<application>/<scenario>`, takes `sessions_attempted` as the sessions, and adds `bytes_tx` and `bytes_rx` for the bytes. The report form is `rest_report` in the entry. A report from a controller before API 0.9.0 has no `scenarios`, and the check says so. A 0.9.0 report with an empty list counts nothing and fails.

**Shares.** Per basis the configured share of an application is the sum of the weights of its scenarios over all weights. The metrics share is its sessions (or bytes) over all scenarios of the run report. The nDPI share is the sessions (or application payload bytes, both directions) that nDPI names as the application over all labelled sessions of the capture, or all sessions when the labels have no rules and no default. Sessions that nDPI names as no application of the mix, `Unknown` included, form the row `(unmatched)` with a configured share of 0, so unrecognised traffic lowers the other shares and fails on its own when it exceeds the tolerance. A scenario in the run report that the mix does not list fails too. Two applications that accept the same nDPI name are refused.

**Comparison.** Each share, from each source, must lie within the tolerance of the configured share, inclusive and exact (a difference of 2.0 points passes at the default). The report gives for each application and basis the configured share, the share from the metrics, the share from nDPI, both deltas in points, the counts, and a reason in words for each source that failed, for example `http1: the sessions share from the run metrics is 57.00% (57 sessions), 3.00 points below the configured 60.00% (tolerance 2.00 points)`. The entry also carries the catalog series of what nDPI saw, see below.

**Metric series.** Catalog 0.4.0 gave `mix.share_pct`, `mix.abs_error_pp`, and `mix.max_abs_error_pp` the label `source`, so the entry's `metrics` holds the series of the capture with `source` `classifier` and `classifier` set to the classifier that ran (`ndpi`). For each application of the mix, `mix.share_pct` is the nDPI share in percent and `mix.abs_error_pp` its distance from the configured share in percentage points, and `mix.max_abs_error_pp`, with no `application` label, is the largest of them, for example `{"name": "mix.share_pct", "labels": {"application": "http1", "source": "classifier", "classifier": "ndpi"}, "value": 80.0}`. Values have at most six decimals. The row `(unmatched)` is no application and has no series, because its traffic already lowers the shares of the others. An application whose share cannot be told (the capture has no sessions or bytes to count) has none either. The catalog gives a mix series no label for the basis, so one report carries one basis, named in `metrics_basis`: `profile.basis` of the test configuration when the check ran with one, or the only basis that ran. Weights in the labels with `basis: both` leave it open, so `metrics_basis` is `null` and `metrics` is empty. The run metrics side of the check keeps its numbers in `bases` and does not become `source` `metrics` series, because the controller reports those for the run.

**Limits.** A scenario that opens several connections per session, such as `web_browse` with its lookup and page fetch, has more sessions in a capture than in the metrics. Use the bytes basis for it, or give the helper connections their own application. The nDPI side needs a capture that holds the whole run, so a capture with a packet limit or a port filter that leaves out an application fails for that application.

## Transport behavior

A real host shows its operating system in the first packets of every TCP connection. The transport behavior check compares the SYN and the SYN-ACK of every connection with the operating system profile of the host that sent it. It runs when the labels file has a `transport` section or when the command has `--os-profile`, `--os-profiles-dir`, `--transport-strict`, `--client-hops`, or `--server-hops`. [`examples/transport.labels.yaml`](examples/transport.labels.yaml) shows the section for the harness fixtures.

```yaml
transport:
  os_profile: linux6        # profile of every host that hosts does not list, or --os-profile
  hosts: {10.0.0.10: windows11}   # a profile per IP address, IPv4 or IPv6
  profiles_dir: profiles    # relative to the labels file; default the kernel backend profiles
  client_hops: 0            # routers between the clients and the capture point (default 0)
  server_hops: 1            # routers between the servers and the capture point (default 0)
  strict: false             # true also enforces the option order and the SYN window
  expect: {window_scale: 10}   # replaces ttl, mss, window_scale, or syn_window of every profile
```

**Profiles.** The check reads the profile files of the kernel backend ([`engine/fps-backend-kernel/profiles/`](../engine/fps-backend-kernel/profiles), TRN-03) and takes `socket.ttl` and `socket.mss` from them, and from `reference` the window scale, the SYN window, the option order, and the do not fragment flag of the real system. SACK permitted and timestamps follow the option list of `reference`, so windows11 offers no timestamps. Reading the files means that the check and the kernel backend share one source of truth, and a custom directory (`--os-profiles-dir`) can hold more profiles.

**Fields.** For the SYN, from the sender of the first SYN without ACK of each connection (a retransmission counts once), the check compares the IP TTL (or hop limit), the do not fragment flag (IPv4), the MSS, the window scale, SACK permitted, and timestamps. For the SYN-ACK, from the other host, it compares the same fields, but a server answers only an option that the client offered: the SYN-ACK must carry SACK permitted, timestamps, and the window scale when the SYN offered them and the profile has them, and none when the SYN lacked them. A SYN-ACK whose SYN is not in the capture is compared in TTL, do not fragment, and MSS only. The TTL comparison is exact: the profile TTL less the hops, so a TTL that is one too high or one too low fails. A SYN with a malformed option list fails.

**Strict mode.** The Linux kernel decides the order of the options and rounds the SYN window to whole segments, so the kernel backend cannot reproduce the option order of windows11 and macos14 or the SYN window of macos14 (see the limits table of the [Transport](../docs/wiki/Transport.md) page). The check therefore lists both as deviations that do not fail the check, and `strict: true` or `--transport-strict` makes them fail. Use strict mode for captures of real systems and for backends that control the whole header.

**Result.** The check fails when any enforced field of any host differs from its profile, when the capture holds no handshake of a host with a profile, and when a profile file is missing or malformed (exit code 2). The report gives the profiles with the values it compared, each host with its profile and the number of SYNs and SYN-ACKs, `unprofiled_handshakes` for hosts that no profile covers (the check leaves them out), and one entry in `deviations` for each host, role, and field that differs, with the expected value, the observed values with their counts, and whether the field is enforced. A failure reads `host 10.201.0.2 (profile windows11): 20 of 20 SYN packets differ in the IP TTL: the profile expects 128, the capture shows 64 in 20 of them`. The entry carries `realism.transport_deviations` for each enforced field and role that was compared (the label `side` is `client` for the SYN and `server` for the SYN-ACK), summed over the hosts. The option order and the SYN window have a series only in strict mode.

**Limits.** The check sees only what the capture shows. A host behind a NAT or a proxy shows the address and the stack of the middlebox, a capture point after an unknown number of routers needs `client_hops` and `server_hops`, and an operating system whose profile file does not exist is not judged. The committed harness fixtures come from the kernel backend without an OS profile, so they show the defaults of Linux 6 with a window scale of 10 (the harness namespaces allow a larger receive buffer than the 7 of the `linux6` profile), and the example labels replace that one value with `expect`.

## Message counts

A capture can classify as the right application and still lack traffic that the scenario describes: an HTTP request that never got its response, an SMTP dialogue without its message, an IMAP session that fetched nothing. nDPI names a session from its first packets, so the conformance check does not see it. The message count check reads the messages of each session with tshark and compares them with the counts that the `messages` section of the labels expects. It runs when the labels file has that section, and `python3 -m realism decrypted` runs it on the decrypted view, so it counts what is inside TLS. [`examples/decrypted/`](examples/decrypted) shows the section for the four fixtures.

```yaml
messages:
  - protocol: smtp               # http1, http2, smtp, or imap
    match: {server_port: 587}    # the fields of a session rule; without match, every TCP session
    expect:
      commands: {EHLO: 2, RCPT: 2, DATA: 1}   # smtp and imap: by command name
      replies: {250: 6, 354: 1}               # smtp: by reply code, imap: by status (OK)
      messages: 1                             # smtp and imap: RFC 5322 messages
  - protocol: http1
    match: {server_port: 8444}
    expect: {requests: 4, responses: {min: 4, max: 5}}   # a count is a number or a range
```

**Rules.** The first rule whose `match` fits a TCP session takes it, and every session of a rule must show every expected count. A count that the rule does not list is not checked. A count is an exact number, or a range with `min` and `max`, either of which may be left out. Names of commands are not case sensitive.

**What tshark counts.** `http1` counts the packets with an HTTP request method (`requests`) and with a status code (`responses`). `http2` counts the HEADERS frames with a `:method` (`requests`) and with a `:status` (`responses`), so an interim 1xx response counts as one. `smtp` and `imap` count the commands (`smtp.req.command`, `imap.request.command`), the SMTP reply codes and the IMAP completion statuses (an untagged line has none), and the messages for which tshark finds an RFC 5322 `From` header, which is the DATA phase of SMTP and each `BODY[]` literal of IMAP. The runner tells tshark to decode the server ports of the selected sessions as the protocol (`-d tcp.port==P,PROTOCOL`), one run for each protocol of the section.

**Result.** The check fails for every session whose count is outside its range, and the reason names the message: `session 12 (10.0.0.10:40001 to 10.0.1.43:587, messages rule 1 (smtp, server_port 587)): missing smtp message: found 0, expected 1`, or `surplus` when there are too many. It fails too when a rule matches no TCP session, because a rule that selects nothing would pass for ever, and when tshark is missing, so a skipped tool never passes. A tshark that runs and fails stops the command with exit code 2. The entry has the totals of each rule, the first 100 sessions with the failed ones first, and the first 50 reasons with `failures_total`. `decrypted` prints up to five of them under `messages:` next to `the view fails messages`. The entry carries one series, `realism.check_failures` with `check` `messages`, whose value is `failures_total` (catalog 0.6.0, CON-25).

**Limits.** The check counts what tshark parses, so a message that tshark cannot read, such as an SMTP message without a `From` header, does not count. The counts say that the messages are there, not that their content is right, which the content check reads. The example labels use the counts of the committed fixtures, so a change of a module scenario changes them in the same pull request.

## Host identity

A capture can carry the configured number of sessions and still show a population that no real network has: far fewer hosts than configured, or two machines that claim one IP address. The host identity check counts the hosts of the capture and looks for address conflicts. It needs no external tool, because it reads the Ethernet frames itself. It runs when the labels file has a `hosts` section (an empty one asks only for the conflict search), or with `--host-identity` or `--host-count`.

```yaml
hosts:
  count: 20000                       # a number or {min, max}; left open when absent
  gateway_macs: [02:00:00:00:00:fe]  # routers whose frames the conflict search leaves out
```

**Hosts.** A host is an IP address that is the source or the destination of an IP packet, or the sender of an ARP request or reply, within one VLAN, so the same address in VLAN 10 and in VLAN 20 is two hosts. Multicast, broadcast, reserved (224.0.0.0 and above), and unspecified addresses are not hosts. A machine with an IPv4 and an IPv6 address counts twice, because the capture does not say that they belong together. Frames of other link types than Ethernet have no MAC address and add only their IP addresses.

**Address conflicts.** An address conflict is an IP address that sends frames with more than one Ethernet source address in one VLAN. The senders are IPv4 and IPv6 packets and ARP senders, so a duplicate address shows in an ARP request as well as in data. An ARP probe (sender 0.0.0.0) and the target of a request send nothing from that address. One MAC with many IP addresses is no conflict, because a router sends the packets of other addresses with its own MAC. A capture on both sides of a router sees the forwarded addresses with two MACs, so `gateway_macs` lists the routers and the search ignores their frames.

**Result.** The check fails for each conflict (`address conflict: 10.0.0.1 sends from 2 MAC addresses (02:00:00:00:0a:01 in 1, 02:00:00:00:0c:03 in 1 frames)`), when the number of hosts lies outside `count` (`the capture holds 2 hosts, the labels expect 3`), when the capture holds no IP host, and when no frame has an Ethernet source address, so a capture that cannot show a conflict never passes. The entry reports `hosts`, `senders` (the hosts that sent a frame with a MAC), `macs`, the first 100 conflicts in address order with the frames of each MAC, and the first 50 reasons with `failures_total`. Its `metrics` hold the one series `realism.check_failures` with `check: host_identity` (catalog 0.7.0, CON-27), whose value is `failures_total`.

**Limits.** The check sees the MAC of the frame, so a NAT or a proxy that rewrites addresses hides a conflict behind its own MAC, and a host that changes its MAC on purpose (a failover pair) shows as a conflict unless the labels list the MAC as a gateway. IPv6 neighbor discovery is read only as IPv6 packets, not as a binding.

## Classifiers

`realism/classifier.py` defines the `Classifier` protocol, which takes a capture and its sessions and returns a protocol name per session. `NdpiClassifier` runs ndpiReader and parses its CSV output by column name, so it reads nDPI 4.2 and later layouts. nDPI 4.2 separates the fields with commas and nDPI 4.14 with `|`, and the parser takes the separator that the header line uses most. nDPI 4.14 ends `ndpiReader -h` with exit code 1 after its usage text, so the classifier accepts the exit codes 0 and 1 for that one call and reads the version from the banner `Welcome to nDPI 4.14.0-...`. CI builds the pinned release, see [Pinned nDPI](#pinned-ndpi).

## Pinned nDPI

Ubuntu 24.04 ships nDPI 4.2.0, which has no HTTP/2 protocol and names an h2c session `Unknown`. [`ci/ndpi.version`](ci/ndpi.version) pins a release and the commit of its tag (`4.14 90090b9a...`), and no other file names them. [`ci/build-ndpi.sh`](ci/build-ndpi.sh) fetches that commit, checks the checkout against the id, builds `libndpi.a` and `ndpiReader`, installs the reader under `/opt/ndpi`, links it into `/usr/local/bin` ahead of the `libndpi-bin` package, and fails when the reader reports another release. [`ci/install-tools.sh`](ci/install-tools.sh) calls it, so the `rlm` and `realism` workflows use one nDPI. The script does nothing but the link when `/opt/ndpi` already holds the pinned reader. `FPS_NDPI_VERSION` and `FPS_NDPI_PREFIX` override the pin and the prefix for a trial run. The build needs `git`, a C compiler, the autotools, `libpcap-dev`, and `libjson-c-dev`, and it takes a few minutes.

## Decrypted view

nDPI and Zeek cannot read inside TLS, so on a capture they name every TLS session `TLS`. `python3 -m realism decrypted` decrypts the capture with tshark and the TLS keys that it embeds (or `--keylog`), writes the plaintext of every TLS session as a capture of plain TCP sessions with the same addresses, and scores that view with the labels file, as `score` scores a capture: nDPI must name the sessions of the view, and with `--parsers` the parsers must report no error. The view is synthetic, built from the output of tshark with correct checksums, so it shows that the decrypted bytes are a protocol that nDPI and the parsers recognize and nothing about the TLS record layer.

```bash
python3 -m realism decrypted --pcap modules/http1/fixtures/http1.pcapng \
  --labels realism/examples/decrypted/http1.labels.yaml --sessions 80
```

| Option | Meaning |
|---|---|
| `--pcap FILE`, `--labels FILE` | The capture and the labels of the *view*: its session rules match the server ports of the TLS sessions (or `--view-port`), and its applications name the protocol inside TLS, for example `{ndpi: [HTTP]}`. A `tls` section would run the encryption check on a view that has no TLS, so leave it out. Without `--view-port`, tshark decodes every TCP `server_port` of the session rules as TLS (`-d tcp.port==P,tls`), so a port other than 443 needs no heuristic (see below) |
| `--keylog FILE` | TLS key log in the NSS format, in addition to the keys that a pcapng capture embeds |
| `--plaintext inside` | Default. The view carries what both sides sent inside TLS |
| `--plaintext upgrade` | For STARTTLS dialogues. The plain start of the session comes first, without the `STARTTLS` (or `STLS`) command and its reply, because nDPI and Zeek stop reading a stream at that command |
| `--starttls-port PORT:PROTOCOL` | Capture port of STARTTLS sessions with their protocol (`smtp`, `imap`, or `pop`), for example `5587:smtp`. Repeat it for several ports (see below) |
| `--view-port P` | Server port of every session of the view, for example the well known port of the protocol when the capture uses another |
| `--sessions N` | Number of TLS sessions that the capture must hold |
| `--view FILE` | Keep the view as a pcap file |
| `--output FILE` | Result path; standard output when absent |
| `--threshold`, `--ndpi-reader`, `--timeout`, `--parsers`, `--tshark`, `--zeek` | As for `score`; `--parsers` defaults to `auto`, which scores the view with every installed parser |

**TLS on ports other than 443.** tshark knows port 443 as TLS and takes any other port for TLS only by a heuristic that needs a record header in the first TCP segment of the client. A ClientHello that arrives in a shorter first segment, for example a write that the kernel splits, was no TLS to the heuristic, so a capture on port 8443 listed 39 of 40 sessions (MOD-23). Without `--view-port` the command now passes `-d tcp.port==P,tls` for the TCP `server_port` of every session rule of the labels to each tshark run that lists or follows the sessions, so the sessions are the same whatever the segmentation. A rule that gives no `server_port` or another transport adds none. With `--view-port` the labels name the port of the view, which the capture does not use, so the command adds none and tshark keeps its heuristic.

**STARTTLS ports.** A STARTTLS session starts in clear text, so decoding its port as TLS (`-d tcp.port==P,tls`) feeds the plain greeting to the TLS dissector from the first byte and loses the session. tshark upgrades such a session itself when the port decodes as the application protocol, because its SMTP, IMAP, and POP3 dissectors hand the stream to TLS after the server accepts `STARTTLS` (`STLS`). `--starttls-port PORT:PROTOCOL` names the capture port and the protocol, and each tshark run then passes `-d tcp.port==P,PROTOCOL`, so a ClientHello that follows the upgrade is found whatever its segmentation (a first segment of 1 to 4 octets included). The option is needed because the labels of a view with `--view-port` name the port of the view (587, 143), not the one of the capture (5587, 1143), so the command cannot take the capture port from them. A port in both `--starttls-port` and the session rules of the labels is decoded as the protocol, not as TLS. The smtp and imap module jobs use `--starttls-port 5587:smtp` and `--starttls-port 1143:imap`. A port that the labels name and that tshark knows as the protocol (143 for IMAP) needs no option. The report format is unchanged.

The result is JSON with the number of TLS sessions, how many decrypted, the reason for each failure, and the full realism report of the view under `report`. The command exits with 0 when every TLS session decrypts, the count matches `--sessions`, and the view passes every check, with 1 when not, and with 2 when an input or a tool fails. A capture without keys fails with `only 0 of N TLS sessions decrypt`. tshark finds the ClientHello of a session by its port or by a heuristic, and the heuristic misses the first TLS session on a port that tshark does not know when the hello spans two segments, so `--sessions` is the guard against a session that never shows up. [`examples/decrypted/`](examples/decrypted) holds the labels of the http1, smtp, imap, and http2 fixtures, whose `decrypted_check.py` scripts the command replaces. Their `messages` sections carry the counts that those scripts checked, see [Message counts](#message-counts): the HTTP requests and responses inside TLS, the SMTP command and reply sequence with its message, and the IMAP command sequence with its `BODY[]` literals. The Zeek `conn.log` check of the imap view and the Zeek message check of the smtp view stay with the modules. `StubClassifier` labels sessions with a function and serves the tests.

## Tests

The tests build every capture in code with [`tests/pcapgen.py`](tests/pcapgen.py), so the repository holds no captured traffic. They replace ndpiReader, tshark, and Zeek with stubs or with fake programs that print fixed output. Several optional modules run the real tools and skip each tool that is not installed. `tests/test_ndpi_reader.py` runs ndpiReader, `tests/test_ndpi_http2.py` runs the nDPI 4.14 reader on h2c sessions and on the http2 fixture, `tests/test_decrypted.py` and `tests/test_decrypted_fixtures.py` run the decrypted command on generated TLS captures and on the committed module fixtures, `tests/test_pinned_ndpi.py` checks the pin and the build script, `tests/test_parser_tools.py` runs tshark and Zeek on a capture with one malformed DNS message and on clean captures, and `tests/test_encryption.py` runs tshark on TLS captures that `tests/tlsgen.py` builds from real handshakes of the Python `ssl` module (with `openssl` for the certificate). `tests/test_content.py` builds every content capture in code, with repeated and with varied bodies, segmented and reordered TCP, and encrypted sessions. `tests/test_structure.py` builds web_browse, ftp, and dns captures in code and reads the first-party scenario files under `modules/`. `tests/test_ks.py` checks the KS tests against known values, and `tests/test_temporal.py` builds captures whose arrivals and sizes come from seeded generators, from the target and from shifted distributions. `tests/test_mix.py` builds captures with known session and byte shares and synthetic run reports, and its optional test runs the real ndpiReader. `tests/test_messages.py` parses the messages section, judges counts with a stub probe, and runs the real tshark on synthetic HTTP, SMTP, and IMAP dialogues and on the four committed fixtures, which it also breaks by cutting a session in half. `tests/test_hosts.py` builds Ethernet frames with chosen MAC addresses, VLAN tags, and ARP packets, and needs no tool. `tests/test_transport.py` builds the handshake of each OS profile from option bytes written out in the test, a handshake with a wrong TTL, MSS, window scale, or option, and runs the check on the committed module fixtures. `tests/test_deliver.py` runs a fake controller on loopback and checks the request, the token sources, and the 409 and 422 answers of `--attach-run`. `tests/test_ja4.py` checks the JA4 code against the worked example of the specification. The `rlm` workflow installs `tshark` from Ubuntu, nDPI 4.14 from source, and `zeek-core` from the package repository of the Zeek project, so CI runs every optional test. The tests of nDPI 4.14 fail instead of skipping on a runner of GitHub Actions that has an older reader.

```bash
python3 -m pip install -r realism/requirements-dev.txt
make lint-rlm test-rlm
```

## Module realism job

`python3 -m realism modules` prints the native modules that a change set selects for the module realism job of the `realism` workflow, as a GitHub Actions matrix or with `--format names` one name per line. `--changed FILE` reads the changed paths, one per line, and `-` reads them from standard input. Without `--changed` every module is selected. [`ci/module-realism.sh`](ci/module-realism.sh) records and scores one module, [`ci/install-tools.sh`](ci/install-tools.sh) installs the tools on a runner, and [`ci/negative_control.py`](ci/negative_control.py) breaks the http1 request line for the negative control. The wiki page Realism describes the job.
