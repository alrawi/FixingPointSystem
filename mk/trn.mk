# SPDX-License-Identifier: GPL-3.0-only
#
# TRN transport backends. The Rust tests of engine/fps-backend-kernel run in
# test-eng and lint-rust; this file adds the namespace capture test of the OS
# profiles and the lint and unit tests of its helpers.

TRN_NETNS_DIR := engine/fps-backend-kernel/tests/netns
# The probe binary of the capture test. sudo drops CARGO_TARGET_DIR, so pass
# TRN_PROBE explicitly when the build used another target directory.
TRN_PROBE ?= $(or $(CARGO_TARGET_DIR),target)/debug/examples/os_profile_probe

# shellcheck runs on the test script, and ruff and mypy on its Python helper
# when they are installed. CI installs them from tests/harness/requirements.txt.
LINT_TARGETS += lint-trn
lint-trn:
	shellcheck $(TRN_NETNS_DIR)/*.sh
	@if command -v ruff >/dev/null 2>&1 && command -v mypy >/dev/null 2>&1; then \
	  echo "ruff format --check $(TRN_NETNS_DIR) && ruff check $(TRN_NETNS_DIR) && mypy --strict $(TRN_NETNS_DIR)/handshakes.py"; \
	  ruff format --check $(TRN_NETNS_DIR) && ruff check $(TRN_NETNS_DIR) && mypy --strict $(TRN_NETNS_DIR)/handshakes.py; \
	else \
	  echo "lint-trn skipped ruff and mypy because they are not installed"; \
	fi

# The unit tests of the handshake checker, which need no root.
TEST_TARGETS += test-trn
test-trn:
	$(PYTHON) -m pytest -q $(TRN_NETNS_DIR)

# Builds the probe as the calling user, so the root only target below needs no
# Rust toolchain.
build-trn-probe:
	$(CARGO) build -p fps-backend-kernel --example os_profile_probe

# The capture test needs root, so make test leaves it out. Run build-trn-probe first.
E2E_TARGETS += e2e-trn
e2e-trn:
	FPS_PROBE="$(TRN_PROBE)" $(TRN_NETNS_DIR)/profiles.sh

.PHONY: build-trn-probe
