# SPDX-License-Identifier: GPL-3.0-only
#
# MOD imap module under modules/imap/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_IMAP_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-imap

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only links. The
# scenarios call the content module, which the manifest requires, so the
# directory links it as well.
LINT_TARGETS += lint-mod-imap
lint-mod-imap:
	mkdir -p $(MOD_IMAP_OUT)/modules
	ln -sfn $(CURDIR)/modules/imap $(MOD_IMAP_OUT)/modules/imap
	ln -sfn $(CURDIR)/modules/content $(MOD_IMAP_OUT)/modules/content
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_IMAP_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, then the realism score
# of the committed fixture, which needs ndpiReader and tshark. The module has
# no Zeek log check, so the realism job only checks that Zeek parses the
# capture without a parser error.
TEST_TARGETS += test-mod-imap
test-mod-imap:
	$(CARGO) test -p fps-module-imap
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_IMAP_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/imap/fixtures/imap.pcapng --labels modules/imap/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/imap/fixtures/imap.pcapng \
	    --labels modules/imap/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_IMAP_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-imap skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness and scores it. It needs root,
# so make test leaves it out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-imap
e2e-mod-imap:
	modules/imap/fixtures/capture.sh $(MOD_IMAP_OUT)/capture

.PHONY: lint-mod-imap test-mod-imap e2e-mod-imap
