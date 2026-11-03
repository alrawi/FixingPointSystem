# SPDX-License-Identifier: GPL-3.0-only
#
# CON contracts.

TEST_TARGETS += test-con
test-con:
	$(PYTHON) tools/contracts/validate.py
	$(PYTHON) -m pytest -q tools/contracts
