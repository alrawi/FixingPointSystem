# Realism

This page describes the realism runner under `realism/`, which the RLM workstream owns. The runner scores a capture against the applications that the traffic was meant to carry and writes a JSON realism report. RLM-01 created the runner with the conformance check, RLM-02 added the parser error check, and RLM-04 added the encryption check, RLM-05 added the content variety check, RLM-08 added the session structure check, and RLM-06 added the temporal distribution check. The [runner README](https://github.com/alrawi/FixingPointSystem/blob/main/realism/README.md) gives the command, the options, and the report fields in full.

## Checks

The [specification](https://github.com/alrawi/FixingPointSystem/blob/main/docs/spec/specification.md#defining-realistic-traffic) defines protocol conformance with two targets. At least 99% of the sessions of every application are classified as that application, and a reference implementation reports zero parser errors. The report carries one entry per check.

| Check | Tools | Passes when | Package |
|---|---|---|---|
| `conformance` | nDPI through `ndpiReader -d` | At least 99% of the labelled sessions of every application carry the intended label | RLM-01 |
| `parser_errors` | tshark expert information and Zeek `weird.log` | At least one parser ran and no parser reported an error | RLM-02 |
| `encryption` | tshark with the exported TLS keys, and JA4 computed by the runner | Every sampled TLS session decrypts and every sampled ClientHello has the JA4 of its client profile | RLM-04 |
| `content` | Payloads reassembled by the runner, no external tool | No application has more than 1% duplicate payloads, and the size distribution agrees with the labels where they give one | RLM-05 |
| `session_structure` | Scenario files and the sessions of the capture, no external tool | Every scenario instance has the connections that its flow graph allows, in the order of its `after` and `spawn` edges | RLM-08 |
| `temporal` | Session start times and payloads read by the runner, no external tool | Every inter arrival time and payload size sample that the labels give a target for agrees with it in a KS test at α (default 0.01), and none has too few observations | RLM-06 |

Every tool runs as a separate process, so the runner links none of their code. We chose the counting rules of the parser error check for three reasons.

1. tshark counts one error per frame, so a frame with several expert items counts once.
2. Zeek counts the weirds of one connection at one packet time once, and it leaves out weirds that come from session state, such as `dns_unmatched_reply`, because they do not mean that a message failed to parse.
3. An application reports the largest count of any one parser, so a fault that both parsers see counts once. A capture with one malformed DNS message therefore reports exactly one parser error.

The parser error check leaves `metrics` empty, because the [metric catalog](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/metrics/catalog.yaml) has no parser error metric yet. [Issue #89](https://github.com/alrawi/FixingPointSystem/issues/89) proposes `realism.parser_errors` with the labels `application` and `parser`.

## Report

[`realism/schema/realism-report.schema.json`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/schema/realism-report.schema.json) defines the report, which is the seam to the controller and the gate suites. A new check adds a definition under `$defs` and one `if` and `then` entry in `$defs.check.allOf`. The report format is at version 0.7.0. Version 0.2.0 added the `parser_errors` entry, version 0.3.0 added `skipped_parsers` to it, version 0.4.0 added the `encryption` entry, version 0.5.0 added the `content` entry, version 0.6.0 added the `session_structure` entry, and version 0.7.0 added the `temporal` entry.

## Seams for tests

Each tool sits behind a protocol with a stub, so unit tests never need the real program.

| Protocol | Real implementations | Stub |
|---|---|---|
| `Classifier` in `classifier.py` | `NdpiClassifier` | `StubClassifier` |
| `ParserRunner` in `parsers.py` | `TsharkRunner`, `ZeekRunner` | `StubParser` |

Component tests replace each program with a small fake executable that prints fixed output, so they reach every error path of the process boundary. The modules `test_ndpi_reader.py` and `test_parser_tools.py` run the real tools and skip each tool that is not installed. Both workflows run [`realism/ci/install-tools.sh`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/ci/install-tools.sh), which installs nDPI and tshark from Ubuntu and one pinned Zeek version from the package repository of the Zeek project, so CI runs every optional test. We run the checks locally with `make lint-rlm test-rlm`.

## Encryption check

The specification asks for real TLS handshakes with common browser fingerprints, checked by decrypting with exported session keys and by computing JA4. RLM-04 implements both in [`realism/encryption.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/encryption.py) and [`realism/ja4.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/ja4.py).

**Configured client profile.** A client profile of the [TLS module](Transport.md#tls-module) fixes the versions, cipher suite order, groups, ALPN, and SNI of a ClientHello, and it carries no fingerprint. The expected fingerprint therefore lives in the labels file, in a `tls` section that maps each profile name to its JA4 (one string or a list), and a session rule with `tls_profile` ties sessions to a profile. A browser profile of TRN-05 documents its JA4 and the labels file of a run copies it. The [runner README](https://github.com/alrawi/FixingPointSystem/blob/main/realism/README.md#encryption) shows the format.

**Sample.** Only sessions whose rule names a `tls_profile` take part. The check takes at most `tls.sample` of them (default 100), spread evenly over the order of their first packet, so the result does not depend on which sessions come first.

**Decryption.** The runner starts tshark once on the capture. tshark applies the keys that a pcapng capture carries in its decryption secrets blocks, which `fps-capture` embeds, and the key log in the NSS format that `--keylog` names, which the agent exports next to the capture. A session counts as decrypted when tshark reads the Finished message of both sides. TLS 1.2 and 1.3 both encrypt that message, and tshark shows it only with the right secrets, so a key log that lacks the handshake secrets, or belongs to other sessions, decrypts nothing. Two other signals do not work. The field `tls.app_data` exists without keys, and `tls.app_data_proto` is set without keys too. The entry reports the decrypted share against `tls.min_decrypted` (default 1) and says whether keys came from the capture, from a sidecar file, or from neither. A TLS 1.3 key log that holds the handshake secrets and lacks the application traffic secrets still counts as decrypted, which the check does not yet catch.

**JA4.** `ja4.py` follows the [FoxIO specification](https://github.com/FoxIO-LLC/ja4/blob/main/technical_details/JA4.md). It ignores GREASE values, takes the version from the highest entry of `supported_versions`, sets the SNI flag from extension 0x0000, counts ciphers and extensions up to 99, takes the first and last character of the first ALPN value (hex when either is not alphanumeric), hashes the sorted cipher suites, and hashes the sorted extensions without SNI and ALPN followed by the signature algorithms in wire order, each truncated to 12 hexadecimal characters of SHA-256. The unit test reproduces the worked example of the specification, `t13d1516h2_8daaf6152771_e5627efa2ab1`, from a ClientHello with GREASE values. The runner reads the ClientHello bytes of each session from tshark, which gives the TCP payload and the reassembled segment data of the frame, parses them itself, and compares its JA4 with the `tls.handshake.ja4` field of tshark 4.2. A disagreement fails the session, because one of the two misreads the hello.

**Verdict.** The check passes when at least `min_decrypted` of the sample decrypts and every sampled ClientHello has a JA4 that its profile lists. A mismatch fails with a reason such as `JA4 t13d1517h2_... is not the JA4 of profile chrome (...): part a (version, SNI flag, cipher and extension counts, ALPN) is t13d1517h2, expected t13d1516h2; cipher suite hash is ..., expected ...`. The check also fails when no session carries a `tls_profile`, and when tshark is missing, so a skipped tool never passes. The entry leaves `metrics` empty until the catalog has metrics for the two shares.

**Tests.** [`realism/tests/tlsgen.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/tests/tlsgen.py) runs a TLS client and server of the Python `ssl` module against each other in memory, writes the flights as TCP segments, and keeps the client secrets as a key log, so no network, root, or dumpcap is needed. The tests cover a matching and a mismatched profile for TLS 1.2 and 1.3, embedded and sidecar keys, a capture without keys, wrong keys, a client without ALPN, and TLS on port 8443. The expected JA4 of the matching fixtures comes from tshark, because the ClientHello of the Python module depends on the installed OpenSSL. The tests skip without tshark or openssl, and the `rlm` workflow has both.

## Content variety check

Traffic that repeats one body, or that always sends the same size, does not look like the application it imitates. RLM-05 implements the check in [`realism/content.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/content.py). It runs when the labels have a `content` section or the command has `--max-duplicate-ratio`, so a labels file without either behaves as before.

**Units.** The runner reassembles each direction of a TCP session itself, from the sequence numbers, the first copy of a byte winning, and it cuts the stream at a hole. It does not use the `capture/` extractor of CAP-01, as RLM-04 decided for the encryption check, so the realism runner stays independent of it. A unit is one HTTP/1 message when the application is HTTP (the name starts with `http`, or the client stream starts with a method), one UDP datagram, or one whole TCP direction otherwise.

**What counts as the same payload.** The unit is hashed with SHA-256 after normalization. For HTTP the hash covers the start line, the headers in wire order with lower case names, and the body without chunked framing, and it leaves out the headers that change per message without changing the content, which are `Date`, `Expires`, `Last-Modified`, `Age`, `ETag`, the conditional headers, cookies, and trace identifiers. We keep the start line and the other headers in the hash because a message with an empty body, such as a GET, would otherwise have the same hash in every session. DNS ignores the transaction identifier. We chose these rules to flag a generator that replays one message, and not a client that asks twice for the same resource, which is why a labels file can drop more headers with `ignore_headers`.

**Duplicate ratio.** For each application and direction, `(units - distinct hashes) / units`. The check fails when the ratio of any application is above `max_duplicate_ratio` (default 0.01, set in the labels or with `--max-duplicate-ratio`). We test each application and not the capture as a whole, so a varied application cannot hide a repeated one. The failure names the most repeated payload, its size and copies, and the sessions that carry it.

**Sizes.** The entry reports count, minimum, maximum, mean, standard deviation, coefficient of variation, and the quantiles p5 to p99 of the unit sizes, per application and direction. A `sizes` entry in the labels gives the expected median and, optionally, the expected coefficient of variation with a relative tolerance (default 25%), and the check fails when the observed values are outside it. Fixed size traffic therefore fails an expected `cv`, and without an expectation the dispersion is only reported.

**Encrypted sessions.** The runner skips sessions with a `tls_profile` and sessions whose stream starts with a TLS record, and it counts them in `skipped_encrypted_sessions`. The encryption check of RLM-04 reads only the Finished messages from tshark and does not expose plaintext, so there is nothing to reuse. The check also fails when no payload could be analysed, so a capture of only TLS sessions never passes. [`realism/tests/test_content.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/tests/test_content.py) builds repeated and varied captures, segmented and reordered TCP streams, and encrypted sessions in code, and the entry leaves `metrics` empty until the catalog has a duplicate ratio metric.

## Session structure check

A scenario instance opens several connections. The `web_browse` scenario of the [specification](https://github.com/alrawi/FixingPointSystem/blob/main/docs/spec/specification.md) resolves a name and then fetches the page, because its page flow runs `after` the lookup flow, and an FTP session opens one control connection and one data connection per transfer, because the control flow spawns each data flow. A generator that drops a connection, repeats one, or runs them in the wrong order makes traffic that no real client makes. RLM-08 implements the check in [`realism/structure.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/structure.py) and [`realism/scenario_graph.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/scenario_graph.py). It runs when the labels have a `structure` section that lists scenario files, or when the command has `--scenario FILE`, so a labels file without either behaves as before.

**What web_browse means here.** The name belongs to two files. The scenario of the specification has two flows, `lookup` (DNS over UDP) and `page` (HTTP, `after: lookup`), and the ENG-07 test of `engine/fps-interp/tests/instances.rs` runs it. The example [`contracts/examples/modules/http1/scenarios/web_browse.yaml`](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/examples/modules/http1/scenarios/web_browse.yaml) has one flow, `browse`, with two requests on one keepalive connection, so it has no ordering to check. The acceptance of RLM-08 needs the first one, and the repository kept it only as a string in a test. [`realism/examples/web_browse.scenario.yaml`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/examples/web_browse.scenario.yaml) is a copy of it for the realism tests, and [`web_browse.labels.yaml`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/examples/web_browse.labels.yaml) shows the labels that turn the check on. The fixtures are generated in `realism/tests/test_structure.py` with pcapgen, because no captured web_browse run exists.

**Graph.** The reader takes each flow as one connection with its transport (`l4`) and its server port, when the `accept` or `connect` action gives a literal or a `{{ vars.name }}` reference. It takes the `after` lists and the `spawn` actions as edges. A `spawn` inside a `loop` repeats `count` times, inside a `branch` it may not happen, and a count that the file does not fix allows any number. The client and the server program of a parent both run `spawn` for one child, and the child counts once.

**Reconstruction.** The runner groups the sessions that match a flow by client address, walks them in order of their first packet, and puts each in the open instance of its client while that instance still has room for the flow. Otherwise it opens a new instance, and a client that was silent for longer than `structure.max_gap_s` also opens one. We chose this slot filling over opening an instance at each root flow, because a page fetch before its lookup must join the instance of that lookup so that the failure names the broken edge and not two incomplete instances. The limits are these. A client address that runs two instances at once cannot be split, clients behind one address look like one, a missing connection shifts the later connections of that client into other instances (the failure says that an instance opened because the one before had no room), flows without a fixed port, such as FTP data connections, are interchangeable and take the connections in spawn order, and two scenarios that share a transport, a port, and a client address cannot be told apart in one capture, so the labels give each its own `match` or the run records one scenario per capture.

**Rules.** Per instance and flow, the number of connections must lie within what the graph allows. Per `after` edge, every connection of the later flow must start no earlier than every connection of the earlier flow completed, where a session completes at its last packet with payload, FIN, or RST. We leave out the closing ACK, because the engine starts the next flow when the close action returns. Per `spawn` edge, every connection of the child must start between the first and the last packet of a connection of its parent. `structure.tolerance_s` (default 0) loosens both. The check also fails when a scenario matches no session and when `expected_instances` disagrees. A failure names the scenario, the instance with its client address and first packet, and the violated edge with both sessions, for example `edge after lookup -> page violated: page (session 1, tcp 10.0.0.10:40001 -> 10.0.1.80:8080) starts at +0.000000 s, before lookup (session 2, udp 10.0.0.10:40000 -> 10.0.1.53:53) completes at +0.030999 s`. The entry leaves `metrics` empty until the catalog has a metric for it.

**The ftp fixture check.** [`modules/ftp/fixtures/structure_check.py`](https://github.com/alrawi/FixingPointSystem/blob/main/modules/ftp/fixtures/structure_check.py) cannot be replaced yet. This check covers its count of one data connection per transfer and its rule that a data connection starts while the control connection is open, and the tests run it on the `mixed_session` scenario. It does not read the 227 and 229 replies, so it cannot check that a data connection goes to the announced port. It does not require that each session ends with a FIN of both sides, and it does not reject active mode commands. The committed `ftp.pcapng` records five scenarios on one address and one control port, so no one scenario file describes it. A replacement needs the fixture script to record one scenario per capture, or a `match` that separates them, and the port and FIN rules would stay in a script or move into a later check.

## Temporal distribution check

Real users arrive at random and fetch objects of very different sizes, and a generator that opens sessions on a metronome, or whose object sizes cluster around one value, does not look like that population. The specification asks for configurable arrival processes, think times, and heavy tailed object sizes, tested against the target distribution with the Kolmogorov Smirnov (KS) test. RLM-06 implements the check in [`realism/temporal.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/temporal.py) and the tests in [`realism/ks.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/ks.py). It runs when the labels have a `temporal` section or the command has `--ks-alpha` or `--ks-min-samples`, so a labels file without either behaves as before.

**What it compares.** Two properties, each per application and for all sessions together.

* `inter_arrival` is the gap in seconds between the first packets of consecutive sessions, in order of those first packets. An application takes the sessions that the labels name as that application, and `overall` takes every session of the capture, labelled or not. A Poisson process of `r` sessions per second has exponential gaps with `mean_ms` of `1000 / r`, and the sum of Poisson processes is Poisson with the summed rate, so one labels file can give a target for each application and one for the whole capture.
* `payload_size` is the size in bytes of each payload unit of the [content check](#content-variety-check), which is one HTTP/1 message with its headers, one UDP datagram, or one whole TCP direction, for the `client` direction, the `server` direction, or `both`. TLS sessions are skipped and counted, as in the content check, because the runner has no plaintext for them.

**Targets.** The labels write a target with the vocabulary of a scenario `delay` in [`contracts/schemas/scenario.schema.json`](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/schemas/scenario.schema.json), so a delay that a scenario uses can be pasted as the target of the arrivals it should produce.

| `dist` | Arrival parameters | Size parameters |
|---|---|---|
| `constant` (or `ms` alone) | `ms`, `tolerance_ms` | `value`, `tolerance` |
| `exponential` | `mean_ms` | `mean` |
| `uniform` | `min_ms`, `max_ms` | `min`, `max` |
| `lognormal` | `mu`, `sigma`, `unit` (`s` or `ms`) | `mu`, `sigma` |
| `pareto` | `alpha`, `scale_ms` | `alpha`, `scale` |
| `empirical` | `sample`, `sample_file`, or `reference` | the same, in bytes |

`mu` is the natural logarithm of the value in the `unit`, and a size `mu` is the logarithm of the size in bytes. A Pareto distribution has the density of the scenario schema, with `scale` as its smallest value. An `empirical` target is a list of values in seconds or bytes, a file of numbers separated by white space or commas, or a reference capture (`reference: {pcap: ..., labels: ...}`, both relative to the labels file) whose own sessions give the sample. The test is then the two sample KS test. The test configuration has no arrival process or size distribution of its own. It holds a load objective and phases, and the scenario schema holds `delay` for think times, so the target distributions are an input of the labels and the check does not read them from the test configuration.

```yaml
temporal:
  alpha: 0.01          # default
  min_samples: 30      # default
  arrivals:
    overall: {dist: exponential, mean_ms: 142.9}
    applications:
      http1: {dist: exponential, mean_ms: 200}
      dns: {dist: uniform, min_ms: 100, max_ms: 900}
  sizes:
    applications:
      http1:
        - {direction: client, dist: lognormal, mu: 5.5, sigma: 0.4}
        - {direction: server, dist: pareto, alpha: 1.8, scale: 500}
```

**Decision.** A comparison passes when the p value of the KS test is at least α (`--ks-alpha`, `temporal.alpha`, default 0.01, and the option wins). The statistic, the p value, the sample size, and the method go into the report beside a failure reason in words, for example `inter-arrival times of application http1: the KS test rejects exponential (mean 0.2) at alpha 0.01 (statistic 0.2133, p value 0.0012, n 200); the observed median is 0.2412 s, the target median is 0.138629 s`. The p value is exact for up to 100 one sample observations and for two samples with up to 10000 pairs, and it is the asymptotic Kolmogorov distribution with the correction of Stephens above that. We wrote the test in pure Python, as the runner needs only the standard library, and we checked it against the table of critical values and against `scipy.stats` values that the tests state. The specification names α = 0.05 for the gate and the check defaults to 0.01, so the gate suite passes `--ks-alpha 0.05` or sets `temporal.alpha`.

**Too little data.** A comparison with fewer than `min_samples` observations (`--ks-min-samples`, `temporal.min_samples`, default 30) in the capture, or in its reference sample, reports `insufficient_data` with the counts, and it fails the check, because a pass on a handful of points proves nothing. The same holds for an application that has no session. The check also fails when the labels name no target, so asking for it with no target never passes.

**Constants.** A constant has no spread, so the check is exact. The observations must all lie within `tolerance` of the value, the statistic is the larger share of observations below or above that band, and the p value is 1 or 0. Capture timestamps have the resolution of the file, so a constant gap needs a tolerance of at least that resolution.

**Limits.** Each comparison is a test at level α, so a report with many targets rejects about α of them by chance even when the traffic is right. Byte sizes are discrete, which makes the KS test against a continuous target conservative, and ties make the exact p values approximate. The gaps come from the first packet of a session in the capture, so a mirror that drops or reorders frames changes them. The check does not yet compare the think times between the actions of one session, which would need the timing of the actions inside a session.

**Metrics.** The entry reports `realism.ks_statistic` and `realism.ks_p_value` for each application whose property the [metric catalog](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/metrics/catalog.yaml) names, `inter_arrival` for arrivals, `request_bytes` for client payloads, and `response_bytes` for server payloads. The comparisons over all sessions and over both directions have no label in the catalog, so they stay in `comparisons` only.

**Evidence.** [`realism/tests/test_ks.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/tests/test_ks.py) holds the known values of the KS tests and checks that exponential samples reject at the rate of α and shifted ones nearly always. [`realism/tests/test_temporal.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/tests/test_temporal.py) builds pcapgen captures whose arrivals and sizes come from seeded Poisson, uniform, lognormal, and Pareto generators. Captures from the target pass, and captures with a 75% longer mean gap, a metronome, a shifted lognormal size, or a wrong constant fail with their reason.

## Pinned Zeek and skipped parsers

RLM-11 applied decision D12 items 5 and 23.

**One Zeek version.** The file [`realism/ci/zeek.version`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/ci/zeek.version) holds the Zeek package version, now `9.0.1-0`, and no other file names it. `install-tools.sh` installs `zeek-core=<version>` from the openSUSE Build Service repository `security:zeek` for Ubuntu 24.04, and both the `rlm` and the `realism` workflow call the script. The script fails when the repository no longer offers the version or when `/opt/zeek/bin/zeek --version` reports another one. `FPS_ZEEK_VERSION` overrides the file for a trial run. The repository keeps one current Zeek release, so a new Zeek release removes the pinned package and the install step fails. The fix is one edit of `zeek.version` after a check that the new version passes the realism checks. A test in `realism/tests/test_pinned_zeek.py` fails when a workflow names a Zeek version of its own.

**Visible skips.** `--parsers auto` still runs every installed parser. The `parser_errors` entry now carries `skipped_parsers`, a list of `{parser, reason}` for each parser that was not installed, next to the older `missing_parsers` names. The runner prints one `skipped parser` line per entry on standard error, and the job summary row of each module reads, for example, `ran tshark 4.2.2, skipped zeek (not installed, no program at zeek)`. A test that puts only tshark on the search path shows Zeek as skipped. `--parsers none` leaves out the parser error check and its list, and `--parsers tshark,zeek` still stops the run when a named parser is missing.

## Module realism job

RLM-03 added the `realism` workflow in [`.github/workflows/realism.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/realism.yml), which scores fresh traffic of every native module that a pull request touches. It runs three jobs.

| Job | What it does |
|---|---|
| `detect` | Lists the changed paths of the pull request and runs `python3 -m realism modules`, which prints a matrix of the selected modules |
| `module` | Records a fresh capture of each selected module in the QA-01 namespace harness, scores it with nDPI, tshark, and Zeek, and uploads the capture and the report as the artifact `realism-<module>` |
| `negative-control` | Breaks the http1 request line in its own checkout and requires the same run to exit with 1 |

A native module takes part when its directory under `modules/` holds `native/Cargo.toml`, `fixtures/capture.sh`, and `fixtures/expected.yaml`. Today these are `dns` and `http1`. The capture script must accept an output directory, write `<module>.pcapng` there, and take a prebuilt peer from `FPS_FIXTURE_PEER`, as the scripts of MOD-01 and MOD-02 do. A changed path under `modules/<name>/` selects that module. A changed path under `realism/`, `engine/`, or `tests/harness/`, or a change to the root Cargo files, `mk/rlm.mk`, or the workflow, selects every module, because it can change the traffic or the score of each of them. A push to `main` also selects every module.

[`realism/ci/module-realism.sh`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/ci/module-realism.sh) runs one module. It calls the capture script of the module and then scores the capture with `--parsers tshark,zeek`, so a missing parser stops the run with exit code 2 instead of passing. It writes `<module>.realism.json` next to the capture and adds one row per check to the job summary. It exits with 1 when the conformance check or the parser error check fails. We run every module locally as root with `make e2e-rlm-modules`.

The negative control shows that the job can fail. [`realism/ci/negative_control.py`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/ci/negative_control.py) removes the space after the method, so every request starts with a line such as `GET/a HTTP/1.1`. In the first run of the job, nDPI classified none of the 120 sessions as HTTP, so the conformance ratio dropped to 0, and Zeek 9.0.1 reported one parser error in each of the 120 sessions. The same run of the unchanged modules passed both checks with tshark 4.2.2 and Zeek 9.0.1. The script exits with 2 when the renderer no longer contains the lines it edits, so a change to `modules/http1/native/src/message.rs` that moves them fails the control instead of hiding it.
