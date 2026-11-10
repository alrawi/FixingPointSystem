# Gate 1 acceptance suite

This suite automates the exit criterion of Phase 1, an HTTP/1.1 and DNS mix that nDPI classifies at 99% through the controller and the agent. QA-04 added it. The wiki page [Gates](../../../docs/wiki/Gates.md) explains the design.

## Files

| File | Purpose |
|---|---|
| `run.sh` | The suite, which runs the controller, the agent, the harness, and the realism runner and writes the gate report |
| `prove-threshold.sh` | The negative proof, which scores a passing capture again with a threshold above the measured ratios and expects the gate to fail, and then runs `prove-corruption.sh` |
| `prove-corruption.sh` | The second negative proof, which corrupts one HTTP message of a passing capture with `corrupt_capture.py` and expects the gate to fail on the parser error check |
| `corrupt_capture.py` | Copies a capture and overwrites one chunk size line of one chunked HTTP response with a chunk size of 17 hex digits, which keeps every frame length |
| `parser_findings.py` | Writes the parser findings of a run to `parser-findings.json`, `.csv`, and `.pcapng` |
| `test_parser_findings.py` | Unit tests of the findings writer |
| `gate1_report.py` | Decides the verdict from the run report, the metrics, and the realism report, and writes `report.md` and `summary.json` |
| `test_gate1_report.py` | Unit tests of the report generator |
| `test-config.yaml` | The test configuration that the suite stores through `POST /v1/configs`, with `@NAME@` markers for the agent, the rate, and the duration |
| `expected.yaml` | The labels of the realism runner for the ports of the mix |

## Commands

```bash
make build-qa-gate1                 # fps-agent and fps-controller as the calling user
sudo make e2e-qa-gate1              # the suite, then the negative proof
sudo tests/gates/gate1/run.sh target/fps-controller target/debug/fps-agent OUT
sudo env FPS_GATE1_THRESHOLD=0.995 tests/gates/gate1/run.sh --rescore OUT
make lint-qa-gate1 test-qa-gate1    # shellcheck, ruff, mypy, and the unit tests
```

`run.sh` exits with 0 when the gate passes, 1 when a gate check fails, and 2 when the suite cannot run. `--rescore OUT` scores the capture of an earlier run again and rewrites the report without a new run. The suite needs root, `ip`, `tcpdump`, `ss`, `curl`, `tar`, Python 3.10 or newer with PyYAML, `ndpiReader`, `tshark`, and Zeek. `realism/ci/install-tools.sh` installs them on Ubuntu 24.04.

| Variable | Default | Meaning |
|---|---|---|
| `FPS_GATE1_THRESHOLD` | `0.99` | Conformance threshold of every application. A value above 1 always fails |
| `FPS_GATE1_RATE` | `100` | Sessions per second of the client run |
| `FPS_GATE1_STEADY_S` | `20` | Seconds of the steady phase after a ramp of 5 seconds |
| `FPS_GATE1_MIN_SESSIONS` | `100` | Sessions that each application needs in the capture |
| `FPS_REALISM_PARSERS` | `tshark,zeek` | Reference parsers of the parser error check |
| `FPS_NETNS` | `gate1` | Name prefix of the harness namespaces |
| `FPS_GATE1_MGMT_NET` | `10.203.0.0/30` | Management link between the client and server namespaces |
| `FPS_GATE1_REPEAT` | `1` | Runs of the suite in a row. Each run writes to `OUT/run-N`, the loop stops at the first run that does not pass, and `OUT` then holds the files of that run, or of the last run when all pass |

## Output

| File in OUT | Content |
|---|---|
| `report.md` | The gate report with every check and its measured value |
| `summary.json` | The verdict, the threshold, the ratio of each application, and every check |
| `gate1.pcapng` | The capture of the router's client arm |
| `realism.json` | The realism report |
| `run-report.json`, `run-report.csv` | `GET /v1/runs/{id}/report` of the client run in JSON and CSV |
| `run-metrics.json` | `GET /v1/runs/{id}/metrics?view=application` of the client run |
| `server-run-report.json` | The report of the server run |
| `test-config.yaml` | The configuration of the client run as stored |
| `context.json` | Run ids, rate, duration, parsers, the packets that tcpdump dropped, and the commit |
| `parser-findings.json`, `parser-findings.csv` | Every parser finding of the run (up to the first 100 of the realism report) with the parser, the kind, the frame, the session, and the facts that tell whether the capture boundary cut the session, in every run, empty when the parsers found nothing |
| `parser-findings.pcapng` | The frames of the sessions that carry a finding, only when there is one |
| `tcpdump.log` | The statistics of tcpdump, with the packets that the kernel dropped |
| `repeat.json` | With `FPS_GATE1_REPEAT` above 1, the exit code of each run |
| `controller.log`, `client-agent.log`, `server-agent.log` | Logs of the three processes |

## The parser error rule

The parser error check fails the gate when tshark or Zeek reports a finding in the capture, with one stated exception: the gate does not count the Zeek weird `premature_connection_reuse` in a session that holds more than one TCP connection. The realism runner joins two connections with one 5-tuple into one session. The kernel allows such a reuse of a client port when the server side holds the old connection in TIME_WAIT, which happens after the server closes first, as in `http1/single_fetch`. Zeek then raises `premature_connection_reuse` for the new SYN. The check of the captured sessions reads 1337 to 1339 of 1340 for `http1` in such a run, and in the last 86 gate runs of CI (up to 2026-11-10) that happened in 16. The two parser failures of 2026-11-10 (the push of MOD-12 and the first attempt on the CTL-39 branch, each with one finding) are both among those 16 runs, and none of the 70 runs without a merged session had a finding. The first run of the QA-09 branch (CI run 42921139910, 1338 of 1340) listed exactly one finding in `parser-findings.json`: the Zeek weird `premature_connection_reuse` in session 881, which holds two connections. tshark does not react to a reused 5-tuple: it reported nothing in 24 local runs with the real agents and in a run with a narrowed port range and 259 reused sessions.

A malformed packet of tshark in a reused session, a Zeek weird of another name (also one that Zeek merges into the same entry), every finding of a session with one connection, and every finding that no session holds still fail the gate.

The report lists every finding with its session and whether it lies in a reused 5-tuple (`### Findings` in `report.md`, `reused_tuple` in `parser-findings.json`), and the check names the number of weirds that it did not count. `reused_sessions` in `parser-findings.json` lists every reused session of the capture, so a reviewer sees how many there were. The rule counts only findings that `parser_findings.py` listed completely, so a report that the realism runner cut off at 100 entries counts all of them.

`prove-corruption.sh` shows that the gate still fails on a capture with a corrupted HTTP message. It runs after `prove-threshold.sh` in `make e2e-qa-gate1`.
