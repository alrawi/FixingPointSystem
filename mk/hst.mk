# SPDX-License-Identifier: GPL-3.0-only
#
# HST host pools and kernel port setup under engine/fps-hosts/. The fast tests
# of the crate already run in test-eng through the engine crate glob, and
# lint-rust covers its code.

# The kernel port integration tests bring up the QA-01 namespace harness and
# need root, so make test leaves them out. The tests are ignored by default,
# and --ignored runs them one at a time.
E2E_TARGETS += e2e-hst
e2e-hst:
	$(CARGO) test -p fps-hosts --test kernel_port -- --ignored --test-threads=1 --nocapture
