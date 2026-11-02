# Handoff RLM-01 Realism runner and nDPI conformance

| Field | Value |
|---|---|
| Issue | #51 |
| Branch | `wp/RLM-01` |
| Pull request | #73 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-rlm01-7fb0, lease until 2026-11-02T21:35:33Z |
| Last verified commit | the head of `wp/RLM-01` at finish |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Score a capture against expected application labels with nDPI run out of process and write a JSON realism report. The command is `python3 -m realism score --pcap FILE --labels FILE`, and the report schema in `realism/schema/` is the seam that later checks, the controller, and QA extend. Conformance is the share of sessions that the classifier labels as the intended application, compared with the 99% threshold of the specification.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the handoff file, and open the pull request.
- [x] 2. Read pcap and pcapng captures with the standard library and build the session table.
- [x] 3. Load the labels file and add the classifier interface with the stub and the nDPI implementation.
- [x] 4. Score conformance, write the report schema, and add the command line entry point.
- [x] 5. Add tests, the `test-rlm` Makefile block, the `rlm` workflow, and the README.
- [x] 6. Ingest into the wiki, merge `origin/main`, and finish.

## Done and verified

1. Claim card 6587219833 on #51, branch `wp/RLM-01`, and pull request #73 with the `wip` label.
2. `realism/capture.py` reads pcap (both byte orders, micro and nanosecond) and pcapng with the standard library, decodes Ethernet with VLAN tags, Linux cooked, null, and raw IP framing, and groups IPv4 and IPv6 TCP and UDP packets into bidirectional sessions. Verified by `realism/tests/test_capture.py` and the Hypothesis properties in `realism/tests/test_property.py`, which show that arbitrary bytes raise only `CaptureError`.
3. `realism/labels.py` loads JSON or YAML labels, and `realism/classifier.py` holds the `Classifier` protocol, `StubClassifier`, and `NdpiClassifier`. Verified by `test_labels.py` and `test_classifier.py`, which drive a fake ndpiReader through success, failure exit, timeout, and a missing program.
4. `realism/conformance.py`, `realism/report.py`, `realism/cli.py`, `realism/schema/realism-report.schema.json`, and `realism/schema/labels.schema.json`. Verified by `test_conformance.py` and `test_cli.py`. Reports validate against the schema, 99 of 100 sessions pass at 0.99 and 98 of 100 fail, and every metric name and label exists in `contracts/metrics/catalog.yaml`.
5. `make lint-rlm test-rlm` passes with 81 tests, `.github/workflows/rlm.yml` installs `libndpi-bin` and runs the same targets, and `realism/README.md` documents the command. `realism/tests/test_ndpi_reader.py` ran against the real ndpiReader 4.2.0 and classified 4 of 4 HTTP and 3 of 3 DNS sessions in pcap and pcapng.
6. Wiki ingest into Implementation Plan and Testing with a Log entry. Verified by `python3 tools/plan/plan.py lint`, `check`, and `render --check`.

## Remaining

Nothing inside the package. Follow up work that later packages own is listed below.

1. RLM-02 adds parser error checks with Zeek and tshark as new entries in `checks`.
2. The runner does not split a TCP 5-tuple that is reused after a close into two sessions, and it does not reassemble IP fragments. Both matter only for captures that the generator does not produce today.
3. A labels file cannot yet require an application to appear. An application with zero sessions is absent from the report instead of failing it.

## Next action

Wait for the review of pull request #73 and apply requested changes.

## Blockers and open questions

1. The root `Makefile` belongs to OPS, and the catalog paths of RLM-01 list only `realism/`. We added an RLM block above the aggregate targets as [Testing](../wiki/Testing.md#commands) prescribes for a new workstream, and the owner should confirm this choice.
2. The built in accepted names map `http1` to `HTTP` and `dns` to `DNS`. Module packages may need more names, for example `HTTP_Proxy`, and can set them in their labels files.

## Decisions taken in this package

1. The runner needs only the standard library. We wrote our own pcap and pcapng reader instead of adding dpkt or Scapy, because the reader needs about 300 lines and the test captures are built by `realism/tests/pcapgen.py`. PyYAML is optional and loaded only for YAML labels.
2. ndpiReader runs with `-d`, which disables its port based guess. Without it a session on port 80 with opaque payload would count as HTTP. `test_real_ndpi_does_not_guess_from_the_port` pins this.
3. The nDPI CSV is parsed by column name, because later nDPI releases insert columns such as `proto_by_ip` before `server_name_sni`.
4. Sessions join classifier output by a direction independent 5-tuple key, and a session that nDPI omits counts as `Unknown`.
5. The threshold comparison uses exact fractions, so 99 of 100 sessions meet 0.99 without floating point doubt.
6. The report carries no timestamp and no absolute paths, so two runs on the same inputs produce identical bytes.
7. The report schema extends by one `if` and `then` entry per check kind in `$defs.check.allOf`, and an unknown check kind needs only `check`, `passed`, and `metrics`.
8. The workflow is named `rlm.yml` after the workstream, as Testing requires, because RLM-03 owns `realism.yml` for the slow module realism job.

## Pitfalls

1. `mypy --strict` must run in the interpreter that has pytest and Hypothesis installed, or the test modules fail with missing imports. The Makefile runs `$(PYTHON) -m mypy` for that reason.
2. Hypothesis writes a `.hypothesis` directory into the working directory. `realism/tests/conftest.py` moves it under the ignored `realism/.hypothesis`.
3. `pip install ./realism` builds in place and leaves `realism/build/` and an egg-info directory, which `realism/.gitignore` ignores.

## Verify the current state

```bash
python3 -m pip install -r realism/requirements-dev.txt
make lint-rlm test-rlm          # 81 passed; the ndpiReader tests skip without nDPI
sudo apt-get install libndpi-bin && make test-rlm   # the ndpiReader tests run
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:06:06Z | claude-rlm01-7fb0 | claim | | Card 6587219833, fresh branch from origin/main |
| 2026-11-02T20:40:00Z | claude-rlm01-7fb0 | checkpoint | | Runner code for steps 2 to 4, tests pending |
| 2026-11-02T21:00:00Z | claude-rlm01-7fb0 | checkpoint | | Tests, Makefile block, rlm workflow, README |
| 2026-11-02T20:20:00Z | claude-rlm01-7fb0 | correction | | The two checkpoint rows above carry estimated times. The commits 4369052 and 2568477 were made at 20:12:20Z and 20:18:08Z |
| 2026-11-02T20:20:00Z | claude-rlm01-7fb0 | finish | | Wiki ingest, status ready for review |
