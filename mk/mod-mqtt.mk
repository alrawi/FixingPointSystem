# SPDX-License-Identifier: GPL-3.0-only
#
# MOD mqtt module under modules/mqtt/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_MQTT_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-mqtt

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link.
LINT_TARGETS += lint-mod-mqtt
lint-mod-mqtt:
	mkdir -p $(MOD_MQTT_OUT)/modules
	ln -sfn $(CURDIR)/modules/mqtt $(MOD_MQTT_OUT)/modules/mqtt
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_MQTT_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, then the realism score
# of the committed fixture, which needs ndpiReader and tshark.
TEST_TARGETS += test-mod-mqtt
test-mod-mqtt:
	$(CARGO) test -p fps-module-mqtt
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_MQTT_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/mqtt/fixtures/mqtt.pcapng --labels modules/mqtt/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/mqtt/fixtures/mqtt.pcapng \
	    --labels modules/mqtt/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_MQTT_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-mqtt skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness and scores it. It needs root,
# so make test leaves it out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-mqtt
e2e-mod-mqtt:
	modules/mqtt/fixtures/capture.sh $(MOD_MQTT_OUT)/capture

.PHONY: lint-mod-mqtt test-mod-mqtt e2e-mod-mqtt
