# SPDX-License-Identifier: GPL-3.0-only
#
# AGT agent service.

# Capability detection against fake system roots and the agent.proto mirror test.
TEST_TARGETS += test-agt
test-agt:
	$(CARGO) test -p fps-agent
