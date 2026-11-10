# SPDX-License-Identifier: GPL-3.0-only
#
# MOD http2 module under modules/http2/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_HTTP2_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-http2

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link to it.
LINT_TARGETS += lint-mod-http2
lint-mod-http2:
	mkdir -p $(MOD_HTTP2_OUT)/modules
	ln -sfn $(CURDIR)/modules/http2 $(MOD_HTTP2_OUT)/modules/http2
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_HTTP2_OUT)/modules --tests --no-openapi

# The interpreter, catalog, scenario, interop, and golden transcript tests, then
# the realism score of the committed fixture. The score needs tshark and an
# ndpiReader that knows HTTP/2, which nDPI 4.2.0 does not (select a newer one
# with FPS_NDPI_READER), so it is skipped when either is missing.
NDPI_READER_HTTP2 = $(or $(FPS_NDPI_READER),ndpiReader)
TEST_TARGETS += test-mod-http2
test-mod-http2:
	$(CARGO) test -p fps-module-http2
	@if command -v tshark >/dev/null 2>&1 && $(NDPI_READER_HTTP2) -h 2>&1 | grep -qw HTTP2; then \
	  mkdir -p $(MOD_HTTP2_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/http2/fixtures/http2.pcapng --labels modules/http2/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/http2/fixtures/http2.pcapng \
	    --labels modules/http2/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_HTTP2_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-http2 skipped the fixture score because tshark or an ndpiReader that knows HTTP/2 is not installed"; \
	fi

# The 60 s fuzz test of the frame and HPACK parsers. It is ignored in the normal
# test run.
fuzz-mod-http2:
	$(CARGO) test -p fps-module-http2 --release --test fuzz -- --ignored fuzz_60s --nocapture

# Records a fresh capture in the QA-01 harness and scores it. It needs root,
# so make test leaves it out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-http2
e2e-mod-http2:
	modules/http2/fixtures/capture.sh $(MOD_HTTP2_OUT)/capture

.PHONY: lint-mod-http2 test-mod-http2 fuzz-mod-http2 e2e-mod-http2
