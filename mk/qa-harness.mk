# SPDX-License-Identifier: GPL-3.0-only
#
# QA namespace harness under tests/harness/.

# shellcheck runs on the harness scripts, and ruff and mypy on its Python helper
# when they are installed. CI installs them from tests/harness/requirements.txt.
LINT_TARGETS += lint-qa-harness
lint-qa: lint-qa-harness
lint-qa-harness:
	shellcheck tests/harness/*.sh
	@if command -v ruff >/dev/null 2>&1 && command -v mypy >/dev/null 2>&1; then \
	  echo "ruff format --check tests/harness && ruff check tests/harness && mypy --strict tests/harness/pcapng.py"; \
	  ruff format --check tests/harness && ruff check tests/harness && mypy --strict tests/harness/pcapng.py; \
	else \
	  echo "lint-qa skipped ruff and mypy because they are not installed"; \
	fi

# The unit tests of the pcapng helper, which need no root.
TEST_TARGETS += test-qa-harness
test-qa: test-qa-harness
test-qa-harness:
	$(PYTHON) -m pytest -q tests/harness

# The namespace harness self test needs root, so make test leaves it out.
E2E_TARGETS += e2e-qa-harness
e2e-qa: e2e-qa-harness
e2e-qa-harness:
	tests/harness/selftest.sh

.PHONY: lint-qa test-qa e2e-qa
