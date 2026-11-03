# SPDX-License-Identifier: GPL-3.0-only
#
# MOD http1 module under modules/http1/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_HTTP1_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-http1

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link to it.
LINT_TARGETS += lint-mod-http1
lint-mod-http1:
	mkdir -p $(MOD_HTTP1_OUT)/modules
	ln -sfn $(CURDIR)/modules/http1 $(MOD_HTTP1_OUT)/modules/http1
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_HTTP1_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, then the realism
# score of the committed fixture, which needs ndpiReader and tshark.
TEST_TARGETS += test-mod-http1
test-mod-http1:
	$(CARGO) test -p fps-module-http1
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_HTTP1_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/http1/fixtures/http1.pcapng --labels modules/http1/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/http1/fixtures/http1.pcapng \
	    --labels modules/http1/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_HTTP1_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-http1 skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness and scores it. It needs root,
# so make test leaves it out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-http1
e2e-mod-http1:
	modules/http1/fixtures/capture.sh $(MOD_HTTP1_OUT)/capture

.PHONY: lint-mod-http1 test-mod-http1 e2e-mod-http1
