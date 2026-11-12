# SPDX-License-Identifier: GPL-3.0-only
#
# QA Gate 2 acceptance suite under tests/gates/gate2/.

# shellcheck runs on the suite scripts, and ruff and mypy on the Python helpers
# when they are installed. CI installs them from tests/gates/gate2/requirements.txt.
LINT_TARGETS += lint-qa-gate2
lint-qa: lint-qa-gate2
lint-qa-gate2:
	shellcheck tests/gates/gate2/*.sh
	@if command -v ruff >/dev/null 2>&1 && command -v mypy >/dev/null 2>&1; then \
	  echo "ruff format --check tests/gates/gate2 && ruff check tests/gates/gate2 && mypy --strict tests/gates/gate2/*.py"; \
	  ruff format --check tests/gates/gate2 && ruff check tests/gates/gate2 && mypy --strict tests/gates/gate2/*.py; \
	else \
	  echo "lint-qa-gate2 skipped ruff and mypy because they are not installed"; \
	fi

# The unit tests of the report generator and of the mix inputs, which need no
# root. The tests of the mix inputs skip themselves without PyYAML.
TEST_TARGETS += test-qa-gate2
test-qa: test-qa-gate2
test-qa-gate2:
	$(PYTHON) -m pytest -q tests/gates/gate2

# build-qa-gate2 builds the agent, the controller, and the fixture peer of every
# native module as the calling user. e2e-qa-gate2 runs the module stage and the
# mix stage as root with those programs and then proves that the gate fails on
# worse results. The image stage needs Packer, QEMU, and KVM, so it has a target of
# its own, e2e-qa-gate2-image, that make e2e leaves out; the qa-gate2 workflow runs
# it in a job. The suite also needs ip, tcpdump, ss, curl, tar, ndpiReader, tshark,
# Zeek, and PyYAML, which realism/ci/install-tools.sh installs.
QA_GATE2_TARGET_DIR ?= $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),target)
QA_GATE2_CONTROLLER ?= $(QA_GATE2_TARGET_DIR)/fps-controller
QA_GATE2_AGENT ?= $(QA_GATE2_TARGET_DIR)/debug/fps-agent
QA_GATE2_PEERS ?= $(QA_GATE2_TARGET_DIR)/gate2-peers
QA_GATE2_OUT ?= $(QA_GATE2_TARGET_DIR)/gate2
build-qa-gate2:
	$(CARGO) build -p fps-agent
	cd controller && GOWORK=off $(GO) build -o $(abspath $(QA_GATE2_CONTROLLER)) ./cmd/fps-controller
	tests/gates/gate2/build-peers.sh $(QA_GATE2_PEERS)

E2E_TARGETS += e2e-qa-gate2
e2e-qa: e2e-qa-gate2
e2e-qa-gate2:
	tests/gates/gate2/modules.sh $(QA_GATE2_PEERS) $(QA_GATE2_OUT)
	tests/gates/gate2/mix.sh $(QA_GATE2_CONTROLLER) $(QA_GATE2_AGENT) $(QA_GATE2_OUT)

e2e-qa-gate2-image:
	tests/gates/gate2/image.sh $(QA_GATE2_OUT)

.PHONY: build-qa-gate2 e2e-qa-gate2-image
