# Gate 1 acceptance suite

This suite automates the exit criterion of Phase 1, an HTTP/1.1 and DNS mix that nDPI classifies at 99% through the controller and the agent. QA-04 added it. The wiki page [Gates](../../../docs/wiki/Gates.md) explains the design.

## Files

| File | Purpose |
|---|---|
| `run.sh` | The suite, which runs the controller, the agent, the harness, and the realism runner and writes the gate report |
| `prove-threshold.sh` | The negative proof, which scores a passing capture again with a threshold above the measured ratios and expects the gate to fail |
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
| `context.json` | Run ids, rate, duration, parsers, and the commit |
| `controller.log`, `client-agent.log`, `server-agent.log` | Logs of the three processes |
