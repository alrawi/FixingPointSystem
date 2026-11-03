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
