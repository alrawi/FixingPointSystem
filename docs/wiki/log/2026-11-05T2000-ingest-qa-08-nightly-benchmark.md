## [2026-11-05] ingest | QA-08 nightly benchmark on the owner host

QA-08 adds the `nightly` job to `.github/workflows/qa-bench.yml`. It runs `make bench-s1 BENCH_COMPARE_ARGS=--require-same-host` on a self hosted runner labeled `fps-bench` at 02:17 UTC and on request, only for `schedule` and `workflow_dispatch` on `main`, so no pull request and no fork queues for it. The [Benchmarks](Benchmarks.md) page and `bench/README.md` give the owner steps to register the runner and the security caveat. The acceptance item of one scheduled run waits for the owner to register the runner and to merge a baseline recorded on that host.

### Shared page notes

- [Testing](Testing.md), row Performance: the nightly job now exists in `qa-bench.yml` and needs the owner runner.
- [Operations](Operations.md) may link the runner steps in `bench/README.md`, section Registering the owner host.
- `mk/qa-bench.mk` gained `BENCH_COMPARE_ARGS`, an empty default that adds flags to the comparison.
