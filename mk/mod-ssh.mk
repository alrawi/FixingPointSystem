# SPDX-License-Identifier: GPL-3.0-only
#
# MOD ssh module under modules/ssh/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_SSH_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-ssh

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link. The
# module requires no other module.
LINT_TARGETS += lint-mod-ssh
lint-mod-ssh:
	mkdir -p $(MOD_SSH_OUT)/modules
	ln -sfn $(CURDIR)/modules/ssh $(MOD_SSH_OUT)/modules/ssh
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_SSH_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, the tests of the Zeek
# check, then the realism score of the committed fixture, which needs
# ndpiReader and tshark, and the Zeek check, which needs Zeek. The count is the
# number of sessions that capture.sh records with its default of 4 sessions per
# scenario.
TEST_TARGETS += test-mod-ssh
test-mod-ssh:
	$(CARGO) test -p fps-module-ssh
	PYTHONDONTWRITEBYTECODE=1 $(PYTHON) -m unittest discover -s modules/ssh/fixtures
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_SSH_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/ssh/fixtures/ssh.pcapng --labels modules/ssh/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/ssh/fixtures/ssh.pcapng \
	    --labels modules/ssh/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_SSH_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-ssh skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi
	@zeek="$${FPS_ZEEK:-$$(command -v zeek || true)}"; \
	if [ -z "$$zeek" ] && [ -x /opt/zeek/bin/zeek ]; then zeek=/opt/zeek/bin/zeek; fi; \
	if [ -n "$$zeek" ]; then \
	  echo "modules/ssh/fixtures/zeek_check.py modules/ssh/fixtures/ssh.pcapng --sessions 16"; \
	  $(PYTHON) modules/ssh/fixtures/zeek_check.py modules/ssh/fixtures/ssh.pcapng \
	    --zeek "$$zeek" --sessions 16; \
	else \
	  echo "test-mod-ssh skipped the Zeek check because zeek is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness, scores it, and runs the
# session count check and the Zeek check. It needs root, so make test leaves it
# out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-ssh
e2e-mod-ssh:
	modules/ssh/fixtures/capture.sh $(MOD_SSH_OUT)/capture

.PHONY: lint-mod-ssh test-mod-ssh e2e-mod-ssh
