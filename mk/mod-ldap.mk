# SPDX-License-Identifier: GPL-3.0-only
#
# MOD ldap module under modules/ldap/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_LDAP_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-ldap

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link. The
# module requires no other module.
LINT_TARGETS += lint-mod-ldap
lint-mod-ldap:
	mkdir -p $(MOD_LDAP_OUT)/modules
	ln -sfn $(CURDIR)/modules/ldap $(MOD_LDAP_OUT)/modules/ldap
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_LDAP_OUT)/modules --tests --no-openapi

# The interpreter, catalog, golden transcript, and tshark tests, then the
# realism score of the committed fixture, which needs ndpiReader and tshark.
TEST_TARGETS += test-mod-ldap
test-mod-ldap:
	$(CARGO) test -p fps-module-ldap
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_LDAP_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/ldap/fixtures/ldap.pcapng --labels modules/ldap/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/ldap/fixtures/ldap.pcapng \
	    --labels modules/ldap/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_LDAP_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-ldap skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness and scores it. It needs root,
# so make test leaves it out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-ldap
e2e-mod-ldap:
	modules/ldap/fixtures/capture.sh $(MOD_LDAP_OUT)/capture

.PHONY: lint-mod-ldap test-mod-ldap e2e-mod-ldap
