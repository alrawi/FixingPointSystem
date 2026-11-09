# SPDX-License-Identifier: GPL-3.0-only
#
# MOD ftp module under modules/ftp/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_FTP_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-ftp

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only links. The
# scenarios call the content module, which the manifest requires, so the
# directory links it as well.
LINT_TARGETS += lint-mod-ftp
lint-mod-ftp:
	mkdir -p $(MOD_FTP_OUT)/modules
	ln -sfn $(CURDIR)/modules/ftp $(MOD_FTP_OUT)/modules/ftp
	ln -sfn $(CURDIR)/modules/content $(MOD_FTP_OUT)/modules/content
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_FTP_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, the tests of the two
# fixture checks, the session structure check of the committed fixture, which
# needs only the CAP-01 extractor, then the realism score of the fixture, which
# needs ndpiReader and tshark, and the Zeek check, which needs Zeek. The counts
# are the control connections, the transfers, and the RETR and STOR commands
# that capture.sh records with its default of 8 sessions per scenario.
TEST_TARGETS += test-mod-ftp
test-mod-ftp:
	$(CARGO) test -p fps-module-ftp
	PYTHONDONTWRITEBYTECODE=1 $(PYTHON) -m unittest discover -s modules/ftp/fixtures
	$(PYTHON) modules/ftp/fixtures/structure_check.py modules/ftp/fixtures/ftp.pcapng \
	  --controls 40 --transfers 80
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_FTP_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/ftp/fixtures/ftp.pcapng --labels modules/ftp/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/ftp/fixtures/ftp.pcapng \
	    --labels modules/ftp/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_FTP_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-ftp skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi
	@zeek="$${FPS_ZEEK:-$$(command -v zeek || true)}"; \
	if [ -z "$$zeek" ] && [ -x /opt/zeek/bin/zeek ]; then zeek=/opt/zeek/bin/zeek; fi; \
	if [ -n "$$zeek" ]; then \
	  echo "modules/ftp/fixtures/zeek_check.py modules/ftp/fixtures/ftp.pcapng --transfers 80 --file-transfers 64"; \
	  $(PYTHON) modules/ftp/fixtures/zeek_check.py modules/ftp/fixtures/ftp.pcapng \
	    --zeek "$$zeek" --transfers 80 --file-transfers 64; \
	else \
	  echo "test-mod-ftp skipped the Zeek check because zeek is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness, scores it, and runs the
# structure check and the Zeek check. It needs root, so make test leaves it
# out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-ftp
e2e-mod-ftp:
	modules/ftp/fixtures/capture.sh $(MOD_FTP_OUT)/capture

.PHONY: lint-mod-ftp test-mod-ftp e2e-mod-ftp
