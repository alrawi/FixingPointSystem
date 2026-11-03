# SPDX-License-Identifier: GPL-3.0-only
#
# OPS repository and swarm tooling.

# The work package catalog and the wiki pages generated from it.
LINT_TARGETS += lint-ops
lint-ops:
	$(PYTHON) tools/plan/plan.py check
	$(PYTHON) tools/plan/plan.py render --check
	$(PYTHON) tools/plan/plan.py lint

# The plan tool tests and the workspace probe.
TEST_TARGETS += test-ops
test-ops: test-ops-plan test-ops-workspace

test-ops-plan:
	$(PYTHON) -m pytest -q tools/plan tools/wiki

# Proves that a new empty crate under engine/ joins the Cargo workspace and builds
# without an edit to the root Cargo.toml. The probe runs in a temporary copy of
# the workspace root, so it never touches Cargo.lock or the source tree.
test-ops-workspace:
	tmp="$$(mktemp -d)"; \
	trap 'rm -rf "$$tmp"' EXIT; \
	cp Cargo.toml Cargo.lock "$$tmp/"; \
	for d in spikes engine modules tests/golden/rust; do \
	  mkdir -p "$$tmp/$$(dirname "$$d")" && cp -R "$$d" "$$tmp/$$d"; \
	done; \
	mkdir -p "$$tmp/engine/fps-probe/src"; \
	printf '[package]\nname = "fps-probe"\nversion = "0.0.1"\nedition = "2021"\n' \
	  > "$$tmp/engine/fps-probe/Cargo.toml"; \
	printf '//! Workspace probe.\n' > "$$tmp/engine/fps-probe/src/lib.rs"; \
	CARGO_TARGET_DIR="$$tmp/target" $(CARGO) build --quiet \
	  --manifest-path "$$tmp/Cargo.toml" -p fps-probe; \
	echo "test-ops-workspace passed, engine/fps-probe built without editing Cargo.toml"

.PHONY: test-ops-plan test-ops-workspace
