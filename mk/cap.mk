# SPDX-License-Identifier: GPL-3.0-only
#
# CAP capture compiler. capture/requirements-dev.txt lists the tools. The package uses
# only the standard library and its tests write their own captures, so nothing here
# needs root, a network, or an external capture tool.

LINT_TARGETS += lint-cap
lint-cap:
	$(PYTHON) -m ruff format --check capture
	$(PYTHON) -m ruff check capture
	$(PYTHON) -m mypy --config-file capture/pyproject.toml capture

TEST_TARGETS += test-cap
test-cap:
	$(PYTHON) -m pytest -q capture

.PHONY: lint-cap test-cap
