# SPDX-License-Identifier: GPL-3.0-only
#
# QA Gate 1 acceptance suite under tests/gates/gate1/.

# shellcheck runs on the suite scripts, and ruff and mypy on the report
# generator when they are installed. CI installs them from
# tests/harness/requirements.txt.
LINT_TARGETS += lint-qa-gate1
lint-qa: lint-qa-gate1
lint-qa-gate1:
	shellcheck tests/gates/gate1/*.sh
	@if command -v ruff >/dev/null 2>&1 && command -v mypy >/dev/null 2>&1; then \
	  echo "ruff format --check tests/gates/gate1 && ruff check tests/gates/gate1 && mypy --strict tests/gates/gate1/*.py"; \
	  ruff format --check tests/gates/gate1 && ruff check tests/gates/gate1 && mypy --strict tests/gates/gate1/*.py; \
	else \
	  echo "lint-qa-gate1 skipped ruff and mypy because they are not installed"; \
	fi

# The unit tests of the report generator, which need no root.
TEST_TARGETS += test-qa-gate1
test-qa: test-qa-gate1
test-qa-gate1:
	$(PYTHON) -m pytest -q tests/gates/gate1

# build-qa-gate1 builds the agent and the controller as the calling user, and
# e2e-qa-gate1 runs the suite as root with those binaries and then proves that
# a threshold above the measured ratios fails the gate. The suite also needs
# ip, tcpdump, ss, curl, tar, ndpiReader, tshark, and Zeek, which
# realism/ci/install-tools.sh installs.
QA_GATE1_TARGET_DIR ?= $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),target)
QA_GATE1_CONTROLLER ?= $(QA_GATE1_TARGET_DIR)/fps-controller
QA_GATE1_AGENT ?= $(QA_GATE1_TARGET_DIR)/debug/fps-agent
QA_GATE1_OUT ?= $(QA_GATE1_TARGET_DIR)/gate1
build-qa-gate1:
	$(CARGO) build -p fps-agent
	cd controller && GOWORK=off $(GO) build -o $(abspath $(QA_GATE1_CONTROLLER)) ./cmd/fps-controller

E2E_TARGETS += e2e-qa-gate1
e2e-qa: e2e-qa-gate1
e2e-qa-gate1:
	tests/gates/gate1/run.sh $(QA_GATE1_CONTROLLER) $(QA_GATE1_AGENT) $(QA_GATE1_OUT)
	tests/gates/gate1/prove-threshold.sh $(QA_GATE1_OUT)

.PHONY: build-qa-gate1
