# SPDX-License-Identifier: GPL-3.0-only
#
# Module strike: modules/strike/. Its Rust tests also run in the rust job of
# ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files live under the Cargo target directory, which Git ignores.
MOD_STRIKE_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-strike

# validate.py treats every directory under --modules as one module, so the
# target validates this module through a parent directory that only holds a
# symlink to it.
LINT_TARGETS += lint-mod-strike
lint-mod-strike:
	mkdir -p $(MOD_STRIKE_OUT)/modules
	ln -sfn $(CURDIR)/modules/strike $(MOD_STRIKE_OUT)/modules/strike
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_STRIKE_OUT)/modules --tests --no-openapi

# Scores the committed fixture with nDPI and tshark, so a plain
# cargo test --workspace stays enough for contributors.
TEST_TARGETS += test-mod-strike
test-mod-strike:
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_STRIKE_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/strike/fixtures/strike.pcapng --labels modules/strike/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/strike/fixtures/strike.pcapng \
	    --labels modules/strike/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_STRIKE_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-strike skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness and scores it. Needs root, so
# make test does not depend on it, and it never overwrites the committed
# fixture on its own.
E2E_TARGETS += e2e-mod-strike
e2e-mod-strike:
	modules/strike/fixtures/capture.sh $(MOD_STRIKE_OUT)/capture

.PHONY: lint-mod-strike test-mod-strike e2e-mod-strike
