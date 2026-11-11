## [2026-11-10] ingest | MOD-18 mqtt module

We added the `mqtt` module under `modules/mqtt`. Its crate `fps-module-mqtt` implements 23 template functions for MQTT 3.1.1 and MQTT 5.0 as a native extension (decision D10): the variable byte integer, `CONNECT` and `CONNACK`, `PUBLISH` with the QoS 0, 1, and 2 flows (`PUBACK`, `PUBREC`, `PUBREL`, `PUBCOMP`), `SUBSCRIBE`, `SUBACK`, `UNSUBSCRIBE`, `UNSUBACK`, `PINGREQ`, `PINGRESP`, `DISCONNECT`, the properties of version 5, and a packet reader that checks every packet against the standard. There are five scenarios, `publish_qos0`, `publish_qos1`, `publish_qos2`, `subscribe`, and `keepalive`, and `version` selects 3.1.1 (4) or 5.0 (5). `fixtures/capture.sh` recorded `fixtures/mqtt.pcapng` in the QA-01 harness: 40 sessions with 40 different 5-tuples (one source port per session, the MOD-21 fix), half in each version. nDPI 4.2.0 labels 40 of 40 `MQTT`, and tshark 4.2.2 reports no parser error. The tests run every scenario in both versions between two in-memory peers, flip a bit to prove that a damaged packet fails the session, and compare the bytes at seed 42 with golden files.

The library finding: the Rust MQTT libraries (`rumqttc`, `paho-mqtt`) run a client or a broker on a network loop of their own, and the interpreter needs a writer and a reader that handle one packet when it asks, so the module writes the packets itself and adds no dependency. A packet has a variable length, so a flow reads it with one `expect` of one byte, six `expect` actions of `mqtt.feed(last)` bytes, and a `set` that calls `mqtt.recv`. A read of size 0 is a no-op at runtime, which lets the six reads serve a packet of any length up to four length bytes. The keep alive timing uses absolute time pacing as `sip_rtp.pace` does (MOD-14): `mqtt.pace` moves the due time on by one interval per call, a paused time test runs 200 pings at 30 seconds with exact spacing, and the pings of the capture are 2.000 seconds apart. nDPI 4.14, the version of CI, was not available in the sandbox, so the CI run of the `mod-mqtt` workflow is the proof for that version.

### Shared page notes

- `mk/mod-mqtt.mk` (new file, a coordinator file):

  ```make
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
  ```

  The recipe lines start with a tab. If the Makefile includes `mk/*.mk` by a wildcard, no other change is needed. If it lists them, add `mk/mod-mqtt.mk`.
- `.github/workflows/mod-mqtt.yml` (new file, a coordinator file):

  ```yaml
  # SPDX-License-Identifier: GPL-3.0-only
  #
  # Workflow of the mqtt module under modules/mqtt/. The rust job of ci.yml runs
  # its Rust tests. The module job validates the manifest and scenarios and
  # scores the committed fixture with nDPI and tshark, so nDPI must classify at
  # least 99% of its sessions as MQTT and tshark must report no parser error
  # (MOD-18 acceptance). The realism workflow records a fresh capture of the
  # module in the QA-01 namespace harness and checks it the same way. The job
  # runs on a GitHub hosted runner, never on a self hosted one.
  name: mod-mqtt

  on:
    push:
      branches: [main]
      paths:
        - modules/mqtt/**
        - engine/fps-flow/**
        - Cargo.lock
        - realism/**
        - tools/contracts/**
        - contracts/schemas/**
        - mk/mod-mqtt.mk
        - .github/workflows/mod-mqtt.yml
    pull_request:
      paths:
        - modules/mqtt/**
        - engine/fps-flow/**
        - Cargo.lock
        - realism/**
        - tools/contracts/**
        - contracts/schemas/**
        - mk/mod-mqtt.mk
        - .github/workflows/mod-mqtt.yml

  permissions:
    contents: read

  env:
    GOWORK: "off"

  jobs:
    module:
      runs-on: ubuntu-24.04
      timeout-minutes: 25
      steps:
        - uses: actions/checkout@v4
        - uses: actions/setup-python@v5
          with:
            python-version: "3.12"
        - name: Install nDPI and tshark, which the checks call out of process
          run: realism/ci/install-tools.sh
        - run: python -m pip install -r tools/contracts/requirements.txt
        - run: make lint-mod-mqtt PYTHON=python
        - name: Score the committed fixture
          run: |
            python -m realism score --pcap modules/mqtt/fixtures/mqtt.pcapng \
              --labels modules/mqtt/fixtures/expected.yaml --parsers tshark --output report.json
            python -c 'import json; r = json.load(open("report.json")); print(json.dumps([c.get("applications", c["check"]) for c in r["checks"]], indent=2))'
  ```

- Modules page rows: this package added the `mqtt` row to the table and the section "The mqtt module" to `docs/wiki/Modules.md` (the workstream component page), so the page needs no further edit.
- `Cargo.lock`: this package adds 15 lines for the new crate `fps-module-mqtt`, which has no dependency that the lock file did not already hold. The lock file is part of the PR.
- The realism job: `realism/ci/module-realism.sh` runs `capture.sh` of every touched module, and `capture.sh` of `mqtt` needs only the tools that the other modules need.
- Agent (AGT follow-up): the registry must dispatch `mqtt` calls. `Mqtt` is `Clone`, and a clone of a fresh value starts a new session, so the plain form fits. The exact entry for the `REGISTRATIONS` list of `engine/fps-agent/src/modules.rs`, and the `fps-module-mqtt` dependency in `engine/fps-agent/Cargo.toml`, are:

  ```rust
  Registration {
      name: fps_module_mqtt::MODULE_NAME,
      catalog: || catalog_of!(fps_module_mqtt),
      build: |ctx| {
          if ctx.given.is_some() {
              return Err(BuildError::Settings(
                  "the plan gives settings, but its native functions take none".to_owned(),
              ));
          }
          Ok(Box::new(fps_module_mqtt::Mqtt::default()))
      },
  },
  ```
