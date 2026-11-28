# SPDX-License-Identifier: GPL-3.0-only
#
# SPK Phase 0 spikes.

TEST_TARGETS += test-spk
test-spk:
	$(CARGO) test -p 'fps-spike-*'
	$(PYTHON) -m pytest -q spikes/s4-trex-astf
	$(CARGO) test --release --manifest-path spikes/s5-smoltcp-flow-table/flowtable/Cargo.toml --locked
