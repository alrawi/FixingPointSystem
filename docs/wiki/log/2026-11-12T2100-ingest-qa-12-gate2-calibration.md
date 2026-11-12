## [2026-11-12] ingest | QA-12 calibration in the Gate 2 report

QA-12 makes the Gate 2 mix stage record and show how the throughput calibration moved the client run. `mix.sh` stores `GET /v1/runs/{id}` of the client run as `mix/run.json`, and `gate2_report.py` renders a `Calibration` section beside the byte shares with the last step of `Run.calibration`: step count, time, factor, target, session rate, and per mix entry the bytes per session, the measured flag, and the multiplier. A run with no step, and a last step without multipliers (the session basis waiver), get a sentence that says so. The section is information and does not change the verdict. Six unit tests in `test_gate2_report.py` cover the shown step, no calibration, no multipliers, the fallback, a malformed value, and the unchanged verdict.

Findings.

- `GET /v1/runs/{id}/report` builds its `run` object with the same `runs.ToAPI` (`controller/internal/report/report.go`), so `mix/run-report.json` already carries `run.calibration`. The report reads it from there when `mix/run.json` is absent, as in the output of an earlier run. The separate fetch follows the work package and keeps the file the contract names as the source.
- Not run here: the mix stage itself (it needs root, `ip`, tcpdump and the built controller and agent). The `qa-gate2` workflow is the end to end check.

### Shared page notes

None.
