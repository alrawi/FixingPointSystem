# SPDX-License-Identifier: GPL-3.0-only
#
# RLM realism validation. realism/requirements-dev.txt lists the tools, and the
# optional tests of ndpiReader, tshark, and Zeek skip when the tool is not installed.

LINT_TARGETS += lint-rlm
lint-rlm:
	$(PYTHON) -m ruff format --check realism
	$(PYTHON) -m ruff check realism
	$(PYTHON) -m mypy --config-file realism/pyproject.toml realism

TEST_TARGETS += test-rlm
test-rlm:
	$(PYTHON) -m pytest -q realism

# The module realism job of the realism workflow for every native module. It
# records a fresh capture of each module in the QA-01 harness and scores it with
# nDPI, tshark, and Zeek, so it needs root and make test leaves it out. The
# reports land under the Cargo target directory, which Git ignores.
RLM_MODULES_OUT := $(or $(CARGO_TARGET_DIR),target)/rlm-modules
E2E_TARGETS += e2e-rlm-modules
e2e-rlm-modules:
	@set -e; for m in $$($(PYTHON) -m realism modules --format names); do \
	  echo "realism/ci/module-realism.sh $$m $(RLM_MODULES_OUT)/$$m"; \
	  realism/ci/module-realism.sh $$m $(RLM_MODULES_OUT)/$$m; \
	done

.PHONY: lint-rlm test-rlm e2e-rlm-modules
