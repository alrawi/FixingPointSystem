# Realism runner

The realism runner scores a capture against the applications that the traffic was meant to carry and writes a JSON realism report. This package implements the protocol conformance check of the [specification](../docs/spec/specification.md#defining-realistic-traffic), which passes when at least 99% of the sessions of every application are classified as that application. nDPI is the classifier, and the runner calls its `ndpiReader` program as a separate process. We never link nDPI, because it carries the LGPL-3.0 license.

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
| `--timeout S` | Seconds that ndpiReader may run, 300 by default |

The exit code is 0 when every check passes, 1 when a check fails, and 2 when the capture, the labels file, or ndpiReader cannot be read or run.

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

## Classifiers

`realism/classifier.py` defines the `Classifier` protocol, which takes a capture and its sessions and returns a protocol name per session. `NdpiClassifier` runs ndpiReader and parses its CSV output by column name, so it reads nDPI 4.2 and later layouts. `StubClassifier` labels sessions with a function and serves the tests.

## Tests

The tests build every capture in code with [`tests/pcapgen.py`](tests/pcapgen.py), so the repository holds no captured traffic. They replace ndpiReader with a stub classifier or with a fake program that prints fixed CSV output. One optional module, `tests/test_ndpi_reader.py`, runs the real ndpiReader and skips when it is not installed. The `rlm` workflow installs the `libndpi-bin` package, so CI runs it.

```bash
python3 -m pip install -r realism/requirements-dev.txt
make lint-rlm test-rlm
```
