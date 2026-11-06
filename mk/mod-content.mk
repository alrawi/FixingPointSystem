# SPDX-License-Identifier: GPL-3.0-only
#
# MOD content module under modules/content/. The Rust tests also run in the
# rust job of ci.yml through cargo test --workspace, and lint-rust covers the
# crate. The module has no scenarios and no capture, because it renders
# message bodies for other modules and speaks no protocol of its own.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_CONTENT_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-content

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link to it.
LINT_TARGETS += lint-mod-content
lint-mod-content:
	mkdir -p $(MOD_CONTENT_OUT)/modules
	ln -sfn $(CURDIR)/modules/content $(MOD_CONTENT_OUT)/modules/content
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_CONTENT_OUT)/modules --tests --no-openapi

# The unit tests, the catalog test, and the generator tests, which include the
# duplicate payload ratio over 10,000 bodies per generator.
TEST_TARGETS += test-mod-content
test-mod-content:
	$(CARGO) test -p fps-module-content

.PHONY: lint-mod-content test-mod-content
