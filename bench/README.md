# Benchmark baselines

This directory records benchmark baselines as JSON and fails a run that regresses beyond a stated tolerance. The tools need only the Python 3.10 standard library.

| Path | Purpose |
|---|---|
| `compare.py` | Compares a result file with a baseline file and exits with 1 on a regression |
| `s1_adapter.py` | Converts a Spike S1 results directory into the format below |
| `baselines/` | Reviewed baseline files, one per suite and host |
| `test_compare.py`, `test_s1_adapter.py` | Unit and component tests, which run no benchmark |
| `requirements.txt` | pytest, ruff, and mypy for the checks |

## Commands

```bash
make lint-qa-bench test-qa-bench        # the checks that CI runs
make bench-s1                           # a real S1 run compared with the S1 baseline
python3 bench/s1_adapter.py spikes/s1-engine/results/<label> -o result.json
python3 bench/compare.py result.json bench/baselines/s1-engine-phase0-container.json
python3 bench/compare.py result.json baseline.json --json report.json --require-same-host
```

`compare.py` exits with 0 when every baseline metric holds, with 1 when a metric regressed or is missing from the result, and with 2 when an input file is invalid or the two files name different suites. `--tolerance` sets the relative tolerance for baseline metrics that state none, which is 0.10 by default. A host whose `cpu` or `vcpus` differs from the baseline host prints a note, and `--require-same-host` turns that note into a failure.

`make bench-s1` takes about three minutes. It builds the spike in release mode, runs `spikes/s1-engine/run-bench.sh` with the label in `BENCH_LABEL`, converts the results, and compares them with the baseline in `BENCH_BASELINE`. The converted result and the JSON report land in `target/bench/`. `DUR` shortens each run for a smoke test, for example `DUR=3 make bench-s1`, but such a run says nothing about performance.

## The fps-bench/v1 format

Result files and baseline files share one format. Unknown top level keys are ignored, so a file may carry notes.

```json
{
  "format": "fps-bench/v1",
  "suite": "s1-engine",
  "recorded": "2026-11-02T04:02:22Z",
  "source": "spikes/s1-engine/results/phase0-container",
  "host": {"cpu": "Intel(R) Xeon(R) Processor @ 2.10GHz", "vcpus": 2, "kernel": "6.18.44-fc-v185"},
  "metrics": {
    "cps-interp.sessions_per_s": {"value": 27693.25, "better": "higher", "tolerance": 0.10, "unit": "sessions/s"},
    "cps-interp.latency_p99_us": {"value": 4099, "better": "lower", "tolerance": 0.25, "unit": "us"},
    "cps-interp.sessions_failed": {"value": 0, "better": "lower", "tolerance": 0, "unit": "sessions"}
  }
}
```

| Key | Required | Meaning |
|---|---|---|
| `format` | yes | Always `fps-bench/v1` |
| `suite` | yes | The benchmark suite; a result and its baseline must name the same suite |
| `host` | no | Free form host facts; `cpu` and `vcpus` take part in the host check |
| `recorded`, `source` | no | When and from which raw files the document was made |
| `metrics` | yes | A non empty map from metric name to an entry or to a bare number |
| `metrics.*.value` | yes | A finite number |
| `metrics.*.better` | no | `higher` or `lower`, default `higher` |
| `metrics.*.tolerance` | no | Allowed worsening as a fraction of the baseline value, default 0.10 |
| `metrics.*.abs_tolerance` | no | Allowed worsening in the metric's unit, default 0 |
| `metrics.*.unit` | no | A label for reports |

The baseline owns the policy. The comparison takes `better`, `tolerance`, and `abs_tolerance` from the baseline and only `value` from the result, so a result cannot loosen its own limits. A metric regresses when it is worse than the baseline value by more than `max(tolerance * abs(baseline), abs_tolerance)`. With a 10% tolerance a session rate that drops by 15% fails and one that drops by 5% passes. `abs_tolerance` exists for metrics whose baseline is zero, such as failed sessions, where any relative tolerance allows nothing. Metrics of the result that the baseline lacks are listed as extra and do not fail the run.

## The S1 adapter

`s1_adapter.py` reads `host.txt` and every `*.json` run file of one `spikes/s1-engine/results/<label>/` directory. For each run it emits five metrics.

| Metric | Direction | Tolerance |
|---|---|---|
| `<run>.sessions_per_s` | higher | 10% |
| `<run>.l7_bits_per_s` | higher | 10% |
| `<run>.latency_p50_us` | lower | 15% |
| `<run>.latency_p99_us` | lower | 25% |
| `<run>.sessions_failed` | lower | none, so one failure regresses |

Run files named `<run>-r<N>` are repetitions, and the adapter reports their mean under `<run>`. For each workload with both a `<workload>-interp` and a `<workload>-baseline` run, it adds `<workload>.interp_over_baseline`, the ratio of the two session rates with a 10% tolerance. That ratio depends less on the host than the absolute rates, so it is the metric to watch when hosts differ. The two phase 0 repetitions varied by about 2% in session rate and about 5% in p99 latency, which set the tolerances above.

## Recorded baselines

| File | Suite | Host | Source |
|---|---|---|---|
| `baselines/s1-engine-phase0-container.json` | `s1-engine` | 2 vCPU Firecracker microVM, Intel Xeon at 2.10 GHz, Linux 6.18.44 | `spikes/s1-engine/results/phase0-container/` of 2026-11-02 |
| `baselines/fps-interp-bulk-container.json` | `fps-interp-bulk` | 2 vCPU Firecracker microVM, Intel Xeon at 2.30 GHz, Linux 6.18.44 | `FPS_BENCH_OUT=<file> cargo bench -p fps-interp --bench bulk` of 2026-11-03 |

A test checks that the S1 baseline equals the adapter output of its committed raw files, so a hand edit of a number fails CI.

## Refreshing a baseline

A baseline changes only in a reviewed pull request that names the host and explains the change. We refresh a baseline in six steps.

1. Run the suite on the host of the baseline, or on the new fixed host, with nothing else running. For S1 that is `LABEL=<label> spikes/s1-engine/run-bench.sh`.
2. Compare the new run with the current baseline with `bench/compare.py`, and keep the report for the pull request.
3. Commit the raw results directory, including `host.txt`, so the baseline stays reproducible.
4. Regenerate the baseline with the adapter, for example `python3 bench/s1_adapter.py spikes/s1-engine/results/<label> -o bench/baselines/s1-engine-<label>.json`. Never edit numbers by hand. A tolerance change goes into the adapter, where reviewers see it.
5. When the host changes, add a new baseline file and a row in the table above instead of overwriting the old one, and point `BENCH_BASELINE` and any CI job at the new file.
6. Open the pull request with the host details, the compare report, and the reason for the change, for example a merged optimization, a new host, or an accepted regression with its follow up issue. The reviewer checks that the raw files, the adapter output, and the baseline agree, which `make test-qa-bench` confirms.

A new suite adds its own adapter next to `s1_adapter.py`, with tests, and records its first baseline the same way.
