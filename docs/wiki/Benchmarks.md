# Benchmarks

This page describes how we guard performance with recorded baselines. The directory [`bench/`](https://github.com/alrawi/FixingPointSystem/tree/main/bench) holds the comparison tool, one adapter per benchmark suite, and the reviewed baseline files. QA-03 created the directory and recorded the Spike S1 engine benchmark as the first baseline. The [README](https://github.com/alrawi/FixingPointSystem/blob/main/bench/README.md) gives the full file format and every command.

## How a comparison works

A suite writes its raw output in its own shape. An adapter converts that output into one document of the format `fps-bench/v1`, which maps metric names to values. `bench/compare.py` then reads a result document and a baseline document of the same suite and judges every baseline metric.

| Field of a baseline metric | Meaning |
|---|---|
| `better` | `higher` or `lower` is better |
| `tolerance` | Allowed worsening as a fraction of the baseline value, 10% when absent |
| `abs_tolerance` | Allowed worsening in the metric's own unit, for metrics whose baseline is zero |

A metric regresses when the result is worse than the baseline by more than `max(tolerance * abs(baseline), abs_tolerance)`. The policy lives only in the baseline, so a result file cannot loosen its own limits. A metric that the result lacks also fails the run. The tool exits with 0 on a pass, 1 on a regression, and 2 on an invalid input. With a 10% tolerance a 15% drop in session rate fails and a 5% drop passes, which is the acceptance test of QA-03.

## The S1 engine baseline

The file `bench/baselines/s1-engine-phase0-container.json` comes from the phase 0 run of `spikes/s1-engine/run-bench.sh` on a 2 vCPU Firecracker microVM (Intel Xeon at 2.10 GHz, Linux 6.18.44, rustc 1.97.0) on 2026-11-02. The adapter `bench/s1_adapter.py` emits 43 metrics from it.

The file `bench/baselines/s1-engine-owner-host.json` is the baseline of the nightly job. It comes from run 42231498234 of the `qa-bench` workflow on the owner host, an 8 vCPU Intel Core i7-8705G at 3.10 GHz with Linux 7.0.0-42 and rustc 1.99.1, on 2026-11-06, after the host was tuned as `bench/README.md` describes. On that host the interpreter keeps up with the hand written baseline for keepalive and connection rates (ratios 1.00 and 1.03), and reaches 0.52 of it for 1 MiB bodies, against 0.74 on the container.

| Metric family | Direction | Tolerance |
|---|---|---|
| Session rate and L7 throughput per run | higher | 10% |
| Session latency p50 per run | lower | 15% |
| Session latency p99 per run | lower | 25% |
| Failed sessions per run | lower | none |
| Interpreter over baseline session rate for keepalive, connection rate, and bulk | higher | 10% |

The three ratios are 0.959, 0.989, and 0.739, which match the 4.1%, 1.1%, and 26.1% overheads on the Spike S1 results page. The ratios depend less on the host than absolute rates, so they are the metrics that ENG-05 watches when it reruns the bulk workload against the 25% criterion. A unit test checks that the baseline equals the adapter output of the committed raw files.

## Where benchmarks run

The `test` job of the `qa-bench` workflow runs only the unit and component tests of `bench/`, on Python 3.10 and 3.12 on hosted runners, when `bench/`, the S1 results, or its own files change. It runs no benchmark, because shared runners are too noisy for a 10% tolerance.

The `nightly` job runs `make bench-s1 BENCH_COMPARE_ARGS=--require-same-host` every night at 02:17 UTC and on request, on a self hosted runner of the owner's Linux server with the labels `self-hosted`, `linux`, and `fps-bench` (decision D12 item 6). It compares the run with `bench/baselines/s1-engine-owner-host.json`, a baseline recorded on that same host, and uploads the log, the converted result, and the JSON report as the artifact `s1-nightly-<run id>`. While that baseline file does not exist, the job records a candidate and ends with a warning. A pull request then turns the candidate into the baseline. The host must not track loopback connections, because the suite fills the conntrack table within seconds otherwise, and section Tuning the host of `bench/README.md` shows the setup.

### Owner steps for the runner

The owner registers the runner, and the full steps are in the [README](https://github.com/alrawi/FixingPointSystem/blob/main/bench/README.md#registering-the-owner-host).

1. Install Rust, Python 3.10 or newer, `git`, `make`, and a C linker on the server, and create a dedicated user without sudo for the runner.
2. In Settings, Actions, Runners, choose New self-hosted runner, download the runner as that user, and run `./config.sh --url https://github.com/alrawi/FixingPointSystem --token <TOKEN> --name owner-host --labels fps-bench --unattended`.
3. Install it as a service with `sudo ./svc.sh install <user>` and `sudo ./svc.sh start`.
4. Run the workflow once from the Actions tab, download the candidate artifact, and open the pull request that records `s1-engine-owner-host.json` with the raw results.

### Security caveat

The repository is public, so a self hosted runner must never run a job for a pull request, in particular not for a fork, because the job would run foreign code on the owner's server. The `nightly` job therefore listens only to `schedule` and `workflow_dispatch` on `main`, and the setting "Require approval for all outside collaborators" stays on. Never add a pull request trigger to a job that selects `fps-bench`. The test job and every other job of the repository stay on hosted runners, so no pull request waits for the self hosted runner.

## Refreshing a baseline

A baseline changes only in a reviewed pull request that names the host and explains the change. The author commits the raw results with `host.txt`, regenerates the baseline with the adapter instead of editing numbers, attaches the compare report, and adds a new file when the host changes. The README lists the six steps.
