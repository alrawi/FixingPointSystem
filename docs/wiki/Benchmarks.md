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

| Metric family | Direction | Tolerance |
|---|---|---|
| Session rate and L7 throughput per run | higher | 10% |
| Session latency p50 per run | lower | 15% |
| Session latency p99 per run | lower | 25% |
| Failed sessions per run | lower | none |
| Interpreter over baseline session rate for keepalive, connection rate, and bulk | higher | 10% |

The three ratios are 0.959, 0.989, and 0.739, which match the 4.1%, 1.1%, and 26.1% overheads on the Spike S1 results page. The ratios depend less on the host than absolute rates, so they are the metrics that ENG-05 watches when it reruns the bulk workload against the 25% criterion. A unit test checks that the baseline equals the adapter output of the committed raw files.

## Where benchmarks run

The `qa-bench` workflow runs only the unit and component tests of `bench/`, on Python 3.10 and 3.12, when `bench/`, the S1 results, or its own files change. It runs no benchmark, because shared runners are too noisy for a 10% tolerance. A real run uses `make bench-s1` on a quiet fixed host, and the nightly performance job of the [Testing](Testing.md) page will call the same target once a fixed host exists.

## Refreshing a baseline

A baseline changes only in a reviewed pull request that names the host and explains the change. The author commits the raw results with `host.txt`, regenerates the baseline with the adapter instead of editing numbers, attaches the compare report, and adds a new file when the host changes. The README lists the six steps.
