# SPDX-License-Identifier: GPL-3.0-only
#
# QA benchmark baselines under bench/.

# ruff and mypy on the comparison tool and its adapters, each skipped with a
# notice when it is missing. CI installs both from bench/requirements.txt.
LINT_TARGETS += lint-qa-bench
lint-qa: lint-qa-bench
lint-qa-bench:
	@if command -v ruff >/dev/null 2>&1; then \
	  echo "ruff format --check bench && ruff check bench"; \
	  ruff format --check bench && ruff check bench; \
	else \
	  echo "ruff of lint-qa-bench skipped because it is not installed"; \
	fi
	@if command -v mypy >/dev/null 2>&1; then \
	  echo "mypy --strict bench"; \
	  mypy --strict --cache-dir=/dev/null --python-executable "$$(command -v $(PYTHON))" bench; \
	else \
	  echo "mypy of lint-qa-bench skipped because it is not installed"; \
	fi

# Unit and component tests of compare.py and the S1 adapter. They run no
# benchmark, so they are fast and stable on any runner.
TEST_TARGETS += test-qa-bench
test-qa: test-qa-bench
test-qa-bench:
	$(PYTHON) -m pytest -q bench

# A real S1 benchmark run compared with the recorded baseline. It takes about
# three minutes and needs a quiet host, so make test leaves it out. BENCH_LABEL
# names the results directory, and BENCH_BASELINE picks another baseline file.
# run-bench.sh expects its binary under target/ of the repository, so the
# target drops CARGO_TARGET_DIR for the build. BENCH_COMPARE_ARGS adds flags to
# the comparison, for example --require-same-host on the nightly owner host.
BENCH_LABEL ?= $(shell hostname)-$(shell date -u +%Y%m%d)
BENCH_BASELINE ?= bench/baselines/s1-engine-phase0-container.json
BENCH_OUT ?= target/bench
BENCH_COMPARE_ARGS ?=
bench-s1:
	env -u CARGO_TARGET_DIR LABEL=$(BENCH_LABEL) spikes/s1-engine/run-bench.sh
	mkdir -p $(BENCH_OUT)
	$(PYTHON) bench/s1_adapter.py spikes/s1-engine/results/$(BENCH_LABEL) -o $(BENCH_OUT)/s1-engine.json
	$(PYTHON) bench/compare.py $(BENCH_OUT)/s1-engine.json $(BENCH_BASELINE) --json $(BENCH_OUT)/s1-engine-report.json $(BENCH_COMPARE_ARGS)

.PHONY: lint-qa test-qa bench-s1
