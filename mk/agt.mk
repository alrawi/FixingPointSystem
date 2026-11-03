# SPDX-License-Identifier: GPL-3.0-only
#
# AGT agent service.

# Capability detection against fake system roots, the agent.proto mirror test,
# and the standalone mode over loopback addresses.
TEST_TARGETS += test-agt
test-agt:
	$(CARGO) test -p fps-agent

# The standalone mode runs contracts/examples/plans/http_dns_smoke.json in the
# QA-01 namespace harness, with the server side and the client side in their
# own namespaces. It needs root, so make test leaves it out.
AGT_TARGET_DIR ?= $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),target)
E2E_TARGETS += e2e-agt
e2e-agt:
	$(CARGO) build -p fps-agent
	engine/fps-agent/tests/netns_smoke.sh $(AGT_TARGET_DIR)/debug/fps-agent
