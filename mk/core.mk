# SPDX-License-Identifier: GPL-3.0-only
#
# Repository wide linters. Owned by OPS.

# Rust formatting and clippy cover every crate in the Cargo workspace.
LINT_TARGETS += lint-rust
lint-rust:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings

# Every tracked shell script passes shellcheck.
LINT_TARGETS += lint-shell
lint-shell:
	git ls-files -z '*.sh' | xargs -0 -r shellcheck

# Licenses and advisories of Rust dependencies. CI runs this check in the
# licenses job, so a machine without cargo-deny skips it with a notice.
LINT_TARGETS += lint-deny
lint-deny:
	@if command -v cargo-deny >/dev/null 2>&1; then \
	  echo "$(CARGO) deny check advisories bans licenses sources"; \
	  $(CARGO) deny check advisories bans licenses sources; \
	else \
	  echo "lint-deny skipped because cargo-deny is not installed"; \
	fi
