# SPDX-License-Identifier: GPL-3.0-only
#
# MOD ntp module under modules/ntp/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_NTP_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-ntp

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link to it.
LINT_TARGETS += lint-mod-ntp
lint-mod-ntp:
	mkdir -p $(MOD_NTP_OUT)/modules
	ln -sfn $(CURDIR)/modules/ntp $(MOD_NTP_OUT)/modules/ntp
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_NTP_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, then the realism
# score of the committed fixture, which needs ndpiReader and tshark.
TEST_TARGETS += test-mod-ntp
test-mod-ntp:
	$(CARGO) test -p fps-module-ntp
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_NTP_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/ntp/fixtures/ntp.pcapng --labels modules/ntp/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/ntp/fixtures/ntp.pcapng \
	    --labels modules/ntp/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_NTP_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-ntp skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness and scores it. It needs root,
# so make test leaves it out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-ntp
e2e-mod-ntp:
	modules/ntp/fixtures/capture.sh $(MOD_NTP_OUT)/capture

.PHONY: lint-mod-ntp test-mod-ntp e2e-mod-ntp
