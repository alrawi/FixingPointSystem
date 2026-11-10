# SPDX-License-Identifier: GPL-3.0-only
#
# MOD smb2 module under modules/smb2/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_SMB2_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-smb2

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link.
LINT_TARGETS += lint-mod-smb2
lint-mod-smb2:
	mkdir -p $(MOD_SMB2_OUT)/modules
	ln -sfn $(CURDIR)/modules/smb2 $(MOD_SMB2_OUT)/modules/smb2
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_SMB2_OUT)/modules --tests --no-openapi

# The tests of the native crate, then the realism score of the committed
# fixture, which needs ndpiReader and tshark. The crate tests skip the tshark
# parts without tshark.
TEST_TARGETS += test-mod-smb2
test-mod-smb2:
	$(CARGO) test -p fps-module-smb2
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_SMB2_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/smb2/fixtures/smb2.pcapng --labels modules/smb2/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/smb2/fixtures/smb2.pcapng \
	    --labels modules/smb2/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_SMB2_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-smb2 skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# The 60 s fuzz test of the message readers and the template functions. It is
# ignored in the normal test run.
fuzz-mod-smb2:
	$(CARGO) test -p fps-module-smb2 --release --test fuzz -- --ignored fuzz_60s --nocapture

# Records a fresh capture in the QA-01 harness and scores it. It needs root,
# so make test leaves it out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-smb2
e2e-mod-smb2:
	modules/smb2/fixtures/capture.sh $(MOD_SMB2_OUT)/capture

.PHONY: lint-mod-smb2 test-mod-smb2 fuzz-mod-smb2 e2e-mod-smb2
