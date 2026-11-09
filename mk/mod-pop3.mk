# SPDX-License-Identifier: GPL-3.0-only
#
# MOD pop3 module under modules/pop3/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_POP3_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-pop3

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only links. The
# scenarios call the content module, which the manifest requires, so the
# directory links it as well.
LINT_TARGETS += lint-mod-pop3
lint-mod-pop3:
	mkdir -p $(MOD_POP3_OUT)/modules
	ln -sfn $(CURDIR)/modules/pop3 $(MOD_POP3_OUT)/modules/pop3
	ln -sfn $(CURDIR)/modules/content $(MOD_POP3_OUT)/modules/content
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_POP3_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, then the realism score
# of the committed fixture, which needs ndpiReader and tshark. The module has
# no Zeek log check, so the realism job only checks that Zeek parses the
# capture without a parser error.
TEST_TARGETS += test-mod-pop3
test-mod-pop3:
	$(CARGO) test -p fps-module-pop3
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_POP3_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/pop3/fixtures/pop3.pcapng --labels modules/pop3/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/pop3/fixtures/pop3.pcapng \
	    --labels modules/pop3/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_POP3_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-pop3 skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness and scores it. It needs root,
# so make test leaves it out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-pop3
e2e-mod-pop3:
	modules/pop3/fixtures/capture.sh $(MOD_POP3_OUT)/capture

.PHONY: lint-mod-pop3 test-mod-pop3 e2e-mod-pop3
