# SPDX-License-Identifier: GPL-3.0-only
#
# MOD strike module under modules/strike/. The first slice carries the
# manifest and scenarios only, so the gate is contract validation. Fixture
# scoring and the QA harness job arrive with the native fixture peer.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_STRIKE_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-strike

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link to it.
LINT_TARGETS += lint-mod-strike
lint-mod-strike:
	mkdir -p $(MOD_STRIKE_OUT)/modules
	ln -sfn $(CURDIR)/modules/strike $(MOD_STRIKE_OUT)/modules/strike
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_STRIKE_OUT)/modules --tests --no-openapi

.PHONY: lint-mod-strike
