# SPDX-License-Identifier: GPL-3.0-only
#
# MOD smb2 module under modules/smb2/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# The tests of the native crate. The tshark test skips itself without tshark.
TEST_TARGETS += test-mod-smb2
test-mod-smb2:
	$(CARGO) test -p fps-module-smb2

# The 60 s fuzz test of the message readers and the template functions. It is
# ignored in the normal test run.
fuzz-mod-smb2:
	$(CARGO) test -p fps-module-smb2 --release --test fuzz -- --ignored fuzz_60s --nocapture

.PHONY: test-mod-smb2 fuzz-mod-smb2
