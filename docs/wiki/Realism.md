# Realism

This page describes the realism runner under `realism/`, which the RLM workstream owns. The runner scores a capture against the applications that the traffic was meant to carry and writes a JSON realism report. RLM-01 created the runner with the conformance check, and RLM-02 added the parser error check. The [runner README](https://github.com/alrawi/FixingPointSystem/blob/main/realism/README.md) gives the command, the options, and the report fields in full.

## Checks

The [specification](https://github.com/alrawi/FixingPointSystem/blob/main/docs/spec/specification.md#defining-realistic-traffic) defines protocol conformance with two targets. At least 99% of the sessions of every application are classified as that application, and a reference implementation reports zero parser errors. The report carries one entry per check.

| Check | Tools | Passes when | Package |
|---|---|---|---|
| `conformance` | nDPI through `ndpiReader -d` | At least 99% of the labelled sessions of every application carry the intended label | RLM-01 |
| `parser_errors` | tshark expert information and Zeek `weird.log` | At least one parser ran and no parser reported an error | RLM-02 |

Every tool runs as a separate process, so the runner links none of their code. We chose the counting rules of the parser error check for three reasons.

1. tshark counts one error per frame, so a frame with several expert items counts once.
2. Zeek counts the weirds of one connection at one packet time once, and it leaves out weirds that come from session state, such as `dns_unmatched_reply`, because they do not mean that a message failed to parse.
3. An application reports the largest count of any one parser, so a fault that both parsers see counts once. A capture with one malformed DNS message therefore reports exactly one parser error.

The parser error check leaves `metrics` empty, because the [metric catalog](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/metrics/catalog.yaml) has no parser error metric yet. [Issue #89](https://github.com/alrawi/FixingPointSystem/issues/89) proposes `realism.parser_errors` with the labels `application` and `parser`.

## Report

[`realism/schema/realism-report.schema.json`](https://github.com/alrawi/FixingPointSystem/blob/main/realism/schema/realism-report.schema.json) defines the report, which is the seam to the controller and the gate suites. A new check adds a definition under `$defs` and one `if` and `then` entry in `$defs.check.allOf`. The report format is at version 0.2.0, which added the `parser_errors` entry.

## Seams for tests

Each tool sits behind a protocol with a stub, so unit tests never need the real program.

| Protocol | Real implementations | Stub |
|---|---|---|
| `Classifier` in `classifier.py` | `NdpiClassifier` | `StubClassifier` |
| `ParserRunner` in `parsers.py` | `TsharkRunner`, `ZeekRunner` | `StubParser` |

Component tests replace each program with a small fake executable that prints fixed output, so they reach every error path of the process boundary. The modules `test_ndpi_reader.py` and `test_parser_tools.py` run the real tools and skip each tool that is not installed. The `rlm` workflow installs nDPI and tshark from Ubuntu and Zeek from the package repository of the Zeek project, so CI runs every optional test. We run the checks locally with `make lint-rlm test-rlm`.

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
