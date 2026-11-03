## [2026-11-03] ingest | QA-03 Benchmark baseline comparison

We added `bench/compare.py`, which compares a result with a baseline in the format `fps-bench/v1` and applies per metric tolerances with a direction, and `bench/s1_adapter.py`, which converts a Spike S1 results directory into that format. The Spike S1 phase 0 run is the first baseline in `bench/baselines/s1-engine-phase0-container.json`, with its host details. The new component page [Benchmarks](Benchmarks.md) describes the comparison, the S1 baseline, and the refresh rule, and `bench/README.md` gives the format and the commands. `mk/qa-bench.mk` adds `lint-qa-bench`, `test-qa-bench`, and the manual target `bench-s1`, and `.github/workflows/qa-bench.yml` runs only the unit tests. The evidence is `make test-qa-bench` and `docs/handoff/QA-03.md` on the branch `wp/QA-03`.

## Shared page notes

The coordinator should link [Benchmarks](Benchmarks.md) from Home and the sidebar. On the Testing page, the CI workflows table should gain a row for `qa-bench.yml`, and the targets table should gain the rows below.

| Target | Runs |
|---|---|
| `lint-qa-bench` | ruff and mypy over `bench/`, each skipped with a notice when missing |
| `test-qa-bench` | The unit and component tests of `bench/compare.py` and `bench/s1_adapter.py` |
| `bench-s1` | A real Spike S1 run compared with the S1 baseline, outside `make test` |
