# SPDX-License-Identifier: GPL-3.0-only
#
# MOD hls module under modules/hls/. The Rust tests also run in the rust job of
# ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_HLS_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-hls

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds links to the
# module and to http1, which it requires.
LINT_TARGETS += lint-mod-hls
lint-mod-hls:
	mkdir -p $(MOD_HLS_OUT)/modules
	ln -sfn $(CURDIR)/modules/hls $(MOD_HLS_OUT)/modules/hls
	ln -sfn $(CURDIR)/modules/http1 $(MOD_HLS_OUT)/modules/http1
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_HLS_OUT)/modules --tests --no-openapi

# The crate tests, the tests of the temporal labels, then the realism score of
# the committed fixture, which needs ndpiReader and tshark. The score judges the
# segment cadence and the bytes of every playback through the temporal labels.
TEST_TARGETS += test-mod-hls
test-mod-hls:
	$(CARGO) test -p fps-module-hls
	PYTHONDONTWRITEBYTECODE=1 $(PYTHON) -m unittest discover -s modules/hls/fixtures
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_HLS_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/hls/fixtures/hls.pcapng --labels modules/hls/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/hls/fixtures/hls.pcapng \
	    --labels modules/hls/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_HLS_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-hls skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness and runs the session count check
# and the score. It needs root, so make test leaves it out, and it never
# overwrites the committed fixture.
E2E_TARGETS += e2e-mod-hls
e2e-mod-hls:
	modules/hls/fixtures/capture.sh $(MOD_HLS_OUT)/capture

.PHONY: lint-mod-hls test-mod-hls e2e-mod-hls
