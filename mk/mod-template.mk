# SPDX-License-Identifier: GPL-3.0-only
#
# MOD module template under modules/_template/, the starting point that
# docs/wiki/Module-Development.md describes. The target proves that a copy of the
# template validates, so the guide never starts a reader from a broken module.
# The crate under modules/_template/native/ runs in the rust job of ci.yml
# through cargo test --workspace, and lint-rust covers it.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_TEMPLATE_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-template

# validate.py treats each directory under --modules as a module, and it requires
# the directory name to equal the module name in the manifest. The directory
# _template is not an identifier, so the target validates the template through a
# parent directory that holds only a link named template, the name of the module.
LINT_TARGETS += lint-mod-template
lint-mod-template:
	mkdir -p $(MOD_TEMPLATE_OUT)/modules
	ln -sfn $(CURDIR)/modules/_template $(MOD_TEMPLATE_OUT)/modules/template
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_TEMPLATE_OUT)/modules --tests --no-openapi

.PHONY: lint-mod-template
