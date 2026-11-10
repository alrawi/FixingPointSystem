## [2026-11-10] ingest | MOD-14 SIP with RTP module

We added the `sip_rtp` module under `modules/sip_rtp`. Its crate `fps-module-sip-rtp` implements eleven template functions as a native extension (decision D10): SIP requests and responses (RFC 3261), SDP offers and answers (RFC 3264, RFC 8866), RTP packets for PCMU, PCMA, G.722, G.729, and Opus (RFC 3550, RFC 3551), a pacing function, and the number of packets and the hang up time of a call. There are five scenarios, `call_pcmu`, `call_pcma`, `call_g729`, `call_opus`, and `busy`. A call is a `signaling` flow on UDP port 5060 and a spawned `media` flow. The server spawns the media flow before the ring time, the client spawns it after the `ACK`, both sides send RTP in lock step on one pair of ports, and the client ends the call with a `BYE`. `fixtures/capture.sh` recorded `fixtures/sip_rtp.pcapng` in the QA-01 harness: 15 calls, 27 sessions with 27 different 5-tuples, nDPI 4.2.0 labels all 15 SIP sessions `SIP` and all 12 media sessions `RTP`, tshark 4.2.2 reports no parser error, and all 24 RTP streams have a mean gap within 0.1 ms of the packet time (20.01 to 20.02 ms for 20 ms, 30.02 to 30.04 ms for 30 ms). The tests run every scenario between two in-memory UDP peers on a paused clock, check the RTP gaps exactly, and compare the bytes at seed 42 with golden files.

The acceptance reads "the temporal check accepts RTP inter arrival times at the configured interval, and Zeek logs every call". The temporal check of the realism runner (`realism/temporal.py`) compares the gaps between the starts of sessions and the payload sizes, not the gaps between the packets of one session, so it cannot see RTP inter arrival times. `modules/sip_rtp/fixtures/rtp_check.py` carries the acceptance in the module: it reads every RTP stream with tshark, takes the packet time from the RTP timestamps, and requires a mean gap within 3% and 90% of the gaps within 25%. A per packet inter arrival property in the realism runner is a follow up (see the shared page notes). The Zeek half of the acceptance is `zeek_check.py`, which requires one `INVITE` row in `sip.log` for every call. Zeek was not installed in the sandbox, so `zeek_check.py` is tested against synthetic logs and proven by the CI run. How Zeek 6 or 7 writes the rows of a dialog (which message makes a row, and when it carries the status code) is a guess until that run, and the check is lenient on purpose.

Three findings shaped the scenarios. First, a plain `delay` of the packet time after every packet drifts: the harness showed 21.5 ms for a 20 ms packet time, because every sleep wakes late and the delays add up. The function `sip_rtp.pace` keeps an absolute due time per flow side, so a late wake shortens the next sleep. Second, tshark reports a parser error for every Opus packet with a random first byte, so the Opus payload starts with a valid table of contents byte. Third, when the server spawned the media flow after the ring time, the first RTP packet of the client arrived before the server bound the port and the call failed with a port unreachable, so the server spawns it before the ring time.

The engine limit (AGT-16): the agent refuses a scenario with several flows of which one is UDP, because the instance runner of the runtime opens TCP connections only. The four call scenarios therefore run through the interpreter and `fixture_peer`, and not through the agent. `busy` has one flow and runs in the agent as soon as the registry dispatches `sip_rtp`. The follow up is a datagram opener in the runtime's `InstanceEnv`, built on the Tokio UDP sockets that `fixture_peer` uses, and a registry entry that builds one `SipRtp` per flow side.

### Shared page notes

- `mk/mod-sip.mk` (new file, a coordinator file):

  ```make
  # SPDX-License-Identifier: GPL-3.0-only
  #
  # MOD sip_rtp module under modules/sip_rtp/. The Rust tests also run in the rust
  # job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

  # Scratch files go under the Cargo target directory, which Git ignores.
  MOD_SIP_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-sip_rtp

  # validate.py treats each directory under --modules as a module, so the target
  # validates the module through a parent directory that holds only a link. The
  # module requires no other module.
  LINT_TARGETS += lint-mod-sip
  lint-mod-sip:
  	mkdir -p $(MOD_SIP_OUT)/modules
  	ln -sfn $(CURDIR)/modules/sip_rtp $(MOD_SIP_OUT)/modules/sip_rtp
  	$(PYTHON) tools/contracts/validate.py --modules $(MOD_SIP_OUT)/modules --tests --no-openapi

  # The interpreter, catalog, and golden transcript tests, the tests of the two
  # fixture checks, then the realism score of the committed fixture, which needs
  # ndpiReader and tshark, the RTP interval check, which needs tshark, and the
  # Zeek check, which needs Zeek. The count is the number of calls that
  # capture.sh records with its default of 3 calls per scenario.
  TEST_TARGETS += test-mod-sip
  test-mod-sip:
  	$(CARGO) test -p fps-module-sip-rtp
  	PYTHONDONTWRITEBYTECODE=1 $(PYTHON) -m unittest discover -s modules/sip_rtp/fixtures
  	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
  	  mkdir -p $(MOD_SIP_OUT); \
  	  echo "$(PYTHON) -m realism score --pcap modules/sip_rtp/fixtures/sip_rtp.pcapng --labels modules/sip_rtp/fixtures/expected.yaml --parsers tshark"; \
  	  $(PYTHON) -m realism score --pcap modules/sip_rtp/fixtures/sip_rtp.pcapng \
  	    --labels modules/sip_rtp/fixtures/expected.yaml --parsers tshark \
  	    --output $(MOD_SIP_OUT)/fixture.report.json; \
  	  echo "modules/sip_rtp/fixtures/rtp_check.py modules/sip_rtp/fixtures/sip_rtp.pcapng"; \
  	  $(PYTHON) -I modules/sip_rtp/fixtures/rtp_check.py modules/sip_rtp/fixtures/sip_rtp.pcapng; \
  	else \
  	  echo "test-mod-sip skipped the fixture score and the RTP interval check because ndpiReader or tshark is not installed"; \
  	fi
  	@zeek="$${FPS_ZEEK:-$$(command -v zeek || true)}"; \
  	if [ -z "$$zeek" ] && [ -x /opt/zeek/bin/zeek ]; then zeek=/opt/zeek/bin/zeek; fi; \
  	if [ -n "$$zeek" ]; then \
  	  echo "modules/sip_rtp/fixtures/zeek_check.py modules/sip_rtp/fixtures/sip_rtp.pcapng --calls 15"; \
  	  $(PYTHON) modules/sip_rtp/fixtures/zeek_check.py modules/sip_rtp/fixtures/sip_rtp.pcapng \
  	    --zeek "$$zeek" --calls 15; \
  	else \
  	  echo "test-mod-sip skipped the Zeek check because zeek is not installed"; \
  	fi

  # Records a fresh capture in the QA-01 harness, scores it, and runs the
  # session count check, the RTP interval check, and the Zeek check. It needs
  # root, so make test leaves it out, and it never overwrites the committed
  # fixture.
  E2E_TARGETS += e2e-mod-sip
  e2e-mod-sip:
  	modules/sip_rtp/fixtures/capture.sh $(MOD_SIP_OUT)/capture

  .PHONY: lint-mod-sip test-mod-sip e2e-mod-sip
  ```

  The recipe lines start with a tab. If the Makefile includes `mk/*.mk` by a wildcard, no other change is needed. If it lists them, add `mk/mod-sip.mk`.
- `.github/workflows/mod-sip.yml` (new file, a coordinator file):

  ```yaml
  # SPDX-License-Identifier: GPL-3.0-only
  #
  # Workflow of the sip_rtp module under modules/sip_rtp/. The rust job of ci.yml
  # runs its Rust tests. The module job validates the manifest and scenarios and
  # scores the committed fixture with nDPI, tshark, and Zeek, so nDPI must
  # classify at least 99% of its sessions as SIP or RTP, no parser reports an
  # error, every RTP stream keeps its packet interval, and sip.log holds an
  # INVITE row for every call. The realism workflow records a fresh capture of
  # the module in the QA-01 namespace harness and checks it the same way. The job
  # runs on a GitHub hosted runner, never on a self hosted one.
  name: mod-sip

  on:
    push:
      branches: [main]
      paths:
        - modules/sip_rtp/**
        - realism/**
        - capture/**
        - tools/contracts/**
        - contracts/schemas/**
        - mk/mod-sip.mk
        - .github/workflows/mod-sip.yml
    pull_request:
      paths:
        - modules/sip_rtp/**
        - realism/**
        - capture/**
        - tools/contracts/**
        - contracts/schemas/**
        - mk/mod-sip.mk
        - .github/workflows/mod-sip.yml

  permissions:
    contents: read

  env:
    GOWORK: "off"

  jobs:
    module:
      runs-on: ubuntu-24.04
      timeout-minutes: 30
      steps:
        - uses: actions/checkout@v4
        - uses: actions/setup-python@v5
          with:
            python-version: "3.12"
        - name: Install nDPI, tshark, and Zeek, which the checks call out of process
          run: realism/ci/install-tools.sh
        - run: python -m pip install -r tools/contracts/requirements.txt
        - run: make lint-mod-sip PYTHON=python
        - name: Unit tests of the fixture checks
          run: PYTHONDONTWRITEBYTECODE=1 python -m unittest discover -s modules/sip_rtp/fixtures
        - name: Score the committed fixture
          run: |
            python -m realism score --pcap modules/sip_rtp/fixtures/sip_rtp.pcapng \
              --labels modules/sip_rtp/fixtures/expected.yaml --parsers tshark,zeek --output report.json
            python -c 'import json; r = json.load(open("report.json")); print(json.dumps([c.get("applications", c["check"]) for c in r["checks"]], indent=2))'
        - name: Check the RTP packet interval of the committed fixture
          run: python -I modules/sip_rtp/fixtures/rtp_check.py modules/sip_rtp/fixtures/sip_rtp.pcapng
        - name: Check sip.log of the committed fixture
          run: |
            python modules/sip_rtp/fixtures/zeek_check.py modules/sip_rtp/fixtures/sip_rtp.pcapng --calls 15
  ```

  The module needs no apt package of its own. `realism/ci/install-tools.sh` installs nDPI, tshark, and Zeek, and any other apt package goes through `tools/ci/apt-install.sh`.
- Modules page rows: this package added the `sip_rtp` row to the table and the section "The sip_rtp module" to `docs/wiki/Modules.md` (the workstream component page), so the page needs no further edit.
- `Cargo.lock`: this package adds the crate `fps-module-sip-rtp` with 14 lines. It depends on `fps-flow`, `rand`, and `tokio`, which the workspace already locks, so no new third party package enters the lock file. The lock file is part of the PR.
- Runtime follow up (AGT-16 left the check in place): add a datagram opener to the runtime's `InstanceEnv` and lift the `check_flows` refusal of multi-flow scenarios with a UDP flow in `engine/fps-agent/src/assemble.rs`. `modules/sip_rtp/native/examples/fixture_peer.rs` is a working opener on Tokio UDP sockets: a bound server socket shared by the sessions of its port, a socket per RTP port that is bound when the media flow starts and released when it ends, and a connected client socket with its own source port per flow. The four call scenarios then run in the agent.
- Agent (AGT registry): the registry must dispatch `sip_rtp` calls and build one `SipRtp` per flow side, because the value holds the RTP sequence number, the SSRC, and the pacing state of one side.
- Realism runner (RLM): `realism/temporal.py` has no property for the gaps between the packets inside one session, so "RTP inter arrival times at the configured interval" has no runner check. A follow up should add an inter arrival property per application (the arrival gaps of the packets of a session against a distribution, with the constant distribution that `labels.schema.json` already knows for session gaps). Until then `modules/sip_rtp/fixtures/rtp_check.py` carries the acceptance.
- The realism job: `realism/ci/module-realism.sh` runs `capture.sh` of every touched module. The capture of `sip_rtp` takes about 40 seconds and uses the unique harness name `fpssipfix`.
- nDPI 4.14: the local run used nDPI 4.2.0. The CI runner pins 4.14, and the CI run of this PR is the proof for that version.
