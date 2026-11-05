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

# AGT-02 runs the real fps-controller against the real agent over loopback.
# It needs Go, curl, tar, and python3 but no root, and it stays out of make
# test because it builds both binaries.
AGT_CONTROLLER ?= $(AGT_TARGET_DIR)/fps-controller
test-agt-controller:
	$(CARGO) build -p fps-agent
	cd controller && GOWORK=off $(GO) build -o $(abspath $(AGT_CONTROLLER)) ./cmd/fps-controller
	engine/fps-agent/tests/controller_e2e.sh $(AGT_CONTROLLER) $(AGT_TARGET_DIR)/debug/fps-agent

# AGT-06 runs only the tests of the kernel requirement and the redial policy
# of decision D12 items 3 and 11. test-agt already includes them, so this
# target stays out of TEST_TARGETS and serves a quick local loop.
test-agt-redial:
	$(CARGO) test -p fps-agent --test redial --test exit_codes --test detect
