# Realism runner

The realism runner scores a capture against the applications that the traffic was meant to carry and writes a JSON realism report. It implements the protocol conformance criterion of the [specification](../docs/spec/specification.md#defining-realistic-traffic) with two checks. The conformance check passes when at least 99% of the sessions of every application are classified as that application, and the parser error check passes when no reference parser reports a parser error. nDPI is the classifier, and tshark and Zeek are the reference parsers. The runner calls each of them as a separate process and never links their code.

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

## Classifiers

`realism/classifier.py` defines the `Classifier` protocol, which takes a capture and its sessions and returns a protocol name per session. `NdpiClassifier` runs ndpiReader and parses its CSV output by column name, so it reads nDPI 4.2 and later layouts. `StubClassifier` labels sessions with a function and serves the tests.

## Tests

The tests build every capture in code with [`tests/pcapgen.py`](tests/pcapgen.py), so the repository holds no captured traffic. They replace ndpiReader, tshark, and Zeek with stubs or with fake programs that print fixed output. Two optional modules run the real tools and skip each tool that is not installed. `tests/test_ndpi_reader.py` runs ndpiReader, and `tests/test_parser_tools.py` runs tshark and Zeek on a capture with one malformed DNS message and on clean captures. The `rlm` workflow installs `libndpi-bin` and `tshark` from Ubuntu and `zeek-core` from the package repository of the Zeek project, so CI runs every optional test.

```bash
python3 -m pip install -r realism/requirements-dev.txt
make lint-rlm test-rlm
```

## Module realism job

`python3 -m realism modules` prints the native modules that a change set selects for the module realism job of the `realism` workflow, as a GitHub Actions matrix or with `--format names` one name per line. `--changed FILE` reads the changed paths, one per line, and `-` reads them from standard input. Without `--changed` every module is selected. [`ci/module-realism.sh`](ci/module-realism.sh) records and scores one module, [`ci/install-tools.sh`](ci/install-tools.sh) installs the tools on a runner, and [`ci/negative_control.py`](ci/negative_control.py) breaks the http1 request line for the negative control. The wiki page Realism describes the job.
