# SPDX-License-Identifier: GPL-3.0-only
#
# ENG agent engine.

# Every crate under engine/ is tested by package name, which equals its
# directory name, so a new engine crate joins this target without an edit here.
# lint-rust already covers the engine crates.
ENG_CRATES := $(notdir $(patsubst %/,%,$(dir $(wildcard engine/*/Cargo.toml))))

TEST_TARGETS += test-eng
test-eng:
	$(CARGO) test $(addprefix -p ,$(ENG_CRATES))
	$(PYTHON) -m pytest -q engine/fps-flow/tests
