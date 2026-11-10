# SPDX-License-Identifier: GPL-3.0-only
#
# MOD sip_rtp module under modules/sip_rtp/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_SIP_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-sip_rtp

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link. The
# module requires no other module.
LINT_TARGETS += lint-mod-sip
lint-mod-sip:
	mkdir -p $(MOD_SIP_OUT)/modules
	ln -sfn $(CURDIR)/modules/sip_rtp $(MOD_SIP_OUT)/modules/sip_rtp
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_SIP_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, the tests of the two
# fixture checks, then the realism score of the committed fixture, which needs
# ndpiReader and tshark, the RTP interval check, which needs tshark, and the
# Zeek check, which needs Zeek. The count is the number of calls that
# capture.sh records with its default of 3 calls per scenario.
TEST_TARGETS += test-mod-sip
test-mod-sip:
	$(CARGO) test -p fps-module-sip-rtp
	PYTHONDONTWRITEBYTECODE=1 $(PYTHON) -m unittest discover -s modules/sip_rtp/fixtures
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_SIP_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/sip_rtp/fixtures/sip_rtp.pcapng --labels modules/sip_rtp/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/sip_rtp/fixtures/sip_rtp.pcapng \
	    --labels modules/sip_rtp/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_SIP_OUT)/fixture.report.json; \
	  echo "modules/sip_rtp/fixtures/rtp_check.py modules/sip_rtp/fixtures/sip_rtp.pcapng"; \
	  $(PYTHON) -I modules/sip_rtp/fixtures/rtp_check.py modules/sip_rtp/fixtures/sip_rtp.pcapng; \
	else \
	  echo "test-mod-sip skipped the fixture score and the RTP interval check because ndpiReader or tshark is not installed"; \
	fi
	@zeek="$${FPS_ZEEK:-$$(command -v zeek || true)}"; \
	if [ -z "$$zeek" ] && [ -x /opt/zeek/bin/zeek ]; then zeek=/opt/zeek/bin/zeek; fi; \
	if [ -n "$$zeek" ]; then \
	  echo "modules/sip_rtp/fixtures/zeek_check.py modules/sip_rtp/fixtures/sip_rtp.pcapng --calls 15"; \
	  $(PYTHON) modules/sip_rtp/fixtures/zeek_check.py modules/sip_rtp/fixtures/sip_rtp.pcapng \
	    --zeek "$$zeek" --calls 15; \
	else \
	  echo "test-mod-sip skipped the Zeek check because zeek is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness, scores it, and runs the
# session count check, the RTP interval check, and the Zeek check. It needs
# root, so make test leaves it out, and it never overwrites the committed
# fixture.
E2E_TARGETS += e2e-mod-sip
e2e-mod-sip:
	modules/sip_rtp/fixtures/capture.sh $(MOD_SIP_OUT)/capture

.PHONY: lint-mod-sip test-mod-sip e2e-mod-sip
