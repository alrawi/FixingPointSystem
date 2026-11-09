# SPDX-License-Identifier: GPL-3.0-only
#
# MOD smtp module under modules/smtp/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_SMTP_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-smtp

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only links. The
# scenarios call the content module, which the manifest requires, so the
# directory links it as well.
LINT_TARGETS += lint-mod-smtp
lint-mod-smtp:
	mkdir -p $(MOD_SMTP_OUT)/modules
	ln -sfn $(CURDIR)/modules/smtp $(MOD_SMTP_OUT)/modules/smtp
	ln -sfn $(CURDIR)/modules/content $(MOD_SMTP_OUT)/modules/content
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_SMTP_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, the tests of the Zeek
# check, then the realism score of the committed fixture, which needs ndpiReader
# and tshark, and the Zeek check of the fixture, which needs Zeek. The counts
# are the messages and the attachments that capture.sh records with its
# default of 8 sessions per scenario.
TEST_TARGETS += test-mod-smtp
test-mod-smtp:
	$(CARGO) test -p fps-module-smtp
	PYTHONDONTWRITEBYTECODE=1 $(PYTHON) -m unittest discover -s modules/smtp/fixtures
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_SMTP_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/smtp/fixtures/smtp.pcapng --labels modules/smtp/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/smtp/fixtures/smtp.pcapng \
	    --labels modules/smtp/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_SMTP_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-smtp skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi
	@zeek="$${FPS_ZEEK:-$$(command -v zeek || true)}"; \
	if [ -z "$$zeek" ] && [ -x /opt/zeek/bin/zeek ]; then zeek=/opt/zeek/bin/zeek; fi; \
	if [ -n "$$zeek" ]; then \
	  echo "modules/smtp/fixtures/zeek_check.py modules/smtp/fixtures/smtp.pcapng --messages 64 --attachments 56"; \
	  $(PYTHON) modules/smtp/fixtures/zeek_check.py modules/smtp/fixtures/smtp.pcapng \
	    --zeek "$$zeek" --messages 64 --attachments 56; \
	else \
	  echo "test-mod-smtp skipped the Zeek check because zeek is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness, scores it, and runs the Zeek
# check. It needs root, so make test leaves it out, and it never overwrites the
# committed fixture.
E2E_TARGETS += e2e-mod-smtp
e2e-mod-smtp:
	modules/smtp/fixtures/capture.sh $(MOD_SMTP_OUT)/capture

.PHONY: lint-mod-smtp test-mod-smtp e2e-mod-smtp
