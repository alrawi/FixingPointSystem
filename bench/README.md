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
make bench-s1 BENCH_COMPARE_ARGS=--require-same-host   # what the nightly owner host job runs
python3 bench/s1_adapter.py spikes/s1-engine/results/<label> -o result.json
python3 bench/compare.py result.json bench/baselines/s1-engine-phase0-container.json
python3 bench/compare.py result.json baseline.json --json report.json --require-same-host
```

`compare.py` exits with 0 when every baseline metric holds, with 1 when a metric regressed or is missing from the result, and with 2 when an input file is invalid or the two files name different suites. `--tolerance` sets the relative tolerance for baseline metrics that state none, which is 0.10 by default. A host whose `cpu` or `vcpus` differs from the baseline host prints a note, and `--require-same-host` turns that note into a failure.

`make bench-s1` takes about three minutes. It builds the spike in release mode, runs `spikes/s1-engine/run-bench.sh` with the label in `BENCH_LABEL`, converts the results, and compares them with the baseline in `BENCH_BASELINE`. The converted result and the JSON report land in `target/bench/`. `BENCH_COMPARE_ARGS` adds flags to the comparison. `DUR` shortens each run for a smoke test, for example `DUR=3 make bench-s1`, but such a run says nothing about performance.

## Registering the owner host

Decision D12 item 6 makes the owner's Linux server the benchmark host. A GitHub Actions self hosted runner on that server runs the `nightly` job of `.github/workflows/qa-bench.yml` at 02:17 UTC every night and on request through the Actions tab. The owner does the registration, because it needs admin rights on the repository and on the server. No agent holds the registration token.

### Security caveat

A self hosted runner executes the job's code on the owner's machine with the rights of the runner user. The repository is public, so anyone can open a pull request from a fork. We therefore never run pull request jobs, fork or not, on this runner. The `nightly` job runs only for the events `schedule` and `workflow_dispatch` on `main`, and it carries the guard `github.repository == 'alrawi/FixingPointSystem'`. Keep these rules when you change the workflow.

- Do not add `pull_request`, `pull_request_target`, or `issue_comment` triggers to a job that uses `fps-bench`.
- Under Settings, Actions, General, set "Fork pull request workflows from outside collaborators" to "Require approval for all outside collaborators".
- Run the service as a dedicated user without sudo, without SSH keys, and without access to other projects. Keep secrets out of the job, which needs none.
- Give the runner only the label set below, so no other job selects it by accident.

### What the host needs

| Need | Detail |
|---|---|
| OS | Linux on x86_64 or aarch64 with glibc and systemd, and outbound HTTPS to github.com and the Actions endpoints |
| Rust | A stable toolchain from rustup for the runner user, with `cargo` and `rustc` on its `PATH`, and a C linker (`build-essential`) |
| Python | Python 3.10 or newer as `python3`, with only the standard library needed for the benchmark |
| Tools | `git`, `make`, `curl`, and `tar` |
| Quiet | Nothing else heavy during 02:00 to 03:00 UTC, because the run takes about three minutes and tolerates 10% noise |
| Stable | The same CPU model and vCPU count as when the baseline was recorded, which `--require-same-host` enforces, and a fixed CPU governor such as `performance` |
| Limits | A hard limit of 65536 open files or more, because the script raises the soft limit to the hard limit for 4000 connections |

### Install and register the runner

1. Create the runner user, for example `sudo useradd --create-home --shell /bin/bash fps-runner`, and install rustup as that user.
2. In the repository open Settings, Actions, Runners, New self-hosted runner, and pick Linux with the architecture of the server. GitHub shows the download commands and a registration token that expires after one hour.
3. As `fps-runner`, run the download and extract commands from that page into `~/actions-runner`.
4. Register with the exact label.

   ```bash
   cd ~/actions-runner
   ./config.sh --url https://github.com/alrawi/FixingPointSystem --token <TOKEN> \
     --name owner-host --labels fps-bench --unattended
   ```

   The runner adds the labels `self-hosted`, `linux`, and the architecture on its own, so the job's `runs-on: [self-hosted, linux, fps-bench]` matches only this runner. Do not add the label `fps-bench` to any other machine, because the baseline belongs to one host.
5. Install it as a service that survives reboots, and start it, as root from the runner directory.

   ```bash
   sudo ./svc.sh install fps-runner
   sudo ./svc.sh start
   sudo ./svc.sh status
   ```

   The runner then appears as Idle under Settings, Actions, Runners.
6. Check the setup once. Run the workflow `qa-bench` from the Actions tab (Run workflow on `main`). Without a baseline the first run records a candidate and ends with a warning.

### Record the baseline of the host

The comparison needs a baseline recorded on this same host. The S1 baseline of the container cannot replace it, because `--require-same-host` fails when the CPU or the vCPU count differs.

1. Download the artifact `s1-nightly-<run id>` of the first run. It holds `target/bench/s1-engine.json` and the raw results with `host.txt`.
2. Open a pull request that follows the six steps of [Refreshing a baseline](#refreshing-a-baseline). It commits the raw files as `spikes/s1-engine/results/owner-host-nightly/` and the adapter output as `bench/baselines/s1-engine-owner-host.json`, and it adds a row to the baseline table. Name the host in the description.
3. From the next night the job runs `make bench-s1 BENCH_COMPARE_ARGS=--require-same-host` against that file and uploads `target/bench/` with the text log, the converted result, and `s1-engine-report.json`. A regression fails the job and the run summary shows the table.

To use another baseline file for a manual run, enter its file name under `bench/baselines/` in the Run workflow form.

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
