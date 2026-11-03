# SPDX-License-Identifier: GPL-3.0-only
#
# MOD dns module under modules/dns/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_DNS_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-dns

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link to it.
LINT_TARGETS += lint-mod-dns
lint-mod-dns:
	mkdir -p $(MOD_DNS_OUT)/modules
	ln -sfn $(CURDIR)/modules/dns $(MOD_DNS_OUT)/modules/dns
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_DNS_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, then the realism
# score of the committed fixture, which needs ndpiReader and tshark.
TEST_TARGETS += test-mod-dns
test-mod-dns:
	$(CARGO) test -p fps-module-dns
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_DNS_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/dns/fixtures/dns.pcapng --labels modules/dns/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/dns/fixtures/dns.pcapng \
	    --labels modules/dns/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_DNS_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-dns skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness and scores it. It needs root,
# so make test leaves it out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-dns
e2e-mod-dns:
	modules/dns/fixtures/capture.sh $(MOD_DNS_OUT)/capture

.PHONY: lint-mod-dns test-mod-dns e2e-mod-dns
