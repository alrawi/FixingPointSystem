# SPDX-License-Identifier: GPL-3.0-only
#
# SPK Phase 0 spikes.

TEST_TARGETS += test-spk
test-spk:
	$(CARGO) test -p 'fps-spike-*'
	$(PYTHON) -m pytest -q spikes/s4-trex-astf
