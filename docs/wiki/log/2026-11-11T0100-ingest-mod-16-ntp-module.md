## [2026-11-11] ingest | MOD-16 NTP module

We added the `ntp` module under `modules/ntp/` and a section on it in the component page [Modules](Modules.md), with a row in its module table. The native crate `fps-module-ntp` holds an RFC 5905 codec and the template functions `ntp.query`, `ntp.reply`, and `ntp.field`. The five scenarios are `sntp_query`, `iburst`, `poll_cycle`, `kod_rate`, and `symmetric_peer`. Timestamps come from the `now_ms` argument, which a scenario sets to `now('epoch_ms')` (ENG-25), so tests pin the clock and the golden files record the pinned instant. The fixture holds 30 sessions. nDPI 4.2.0 labels 30 of 30 `NTP`, and tshark 4.2.2 reports no parser error and no malformed packet.

Finding for the coordinator. nDPI 4.2.0 classifies NTP only when one side of the flow uses port 123. We recorded the same scenarios on port 5123 in the harness, and nDPI reported every flow as `Unknown`. The fixture capture therefore uses port 123 on the server, which the harness allows because the server has its own namespace and `capture.sh` runs as root. The tests run over in memory datagrams and need no port. The manifest parameter `server_port` stays for tests and says so.

### Shared page notes

- Workflow `.github/workflows/mod-ntp.yml` (coordinator file). Create it with exactly this content. It installs apt packages only through `tools/ci/apt-install.sh`. The catalog asks for tshark and nDPI and no Zeek check, so the job does not install Zeek, and the `realism` workflow, which installs it through `realism/ci/install-tools.sh`, scores the fresh capture with Zeek too.

```yaml
# SPDX-License-Identifier: GPL-3.0-only
#
# Workflow of the ntp module under modules/ntp/. The rust job of ci.yml runs
# its Rust tests. The module job validates the manifest and scenarios and scores
# the committed fixture with nDPI and tshark. The harness job records a fresh
# capture in the QA-01 namespace harness over UDP sockets and scores it, so
# the fixture stays reproducible.
name: mod-ntp

on:
  push:
    branches: [main]
    paths:
      - modules/ntp/**
      - engine/**
      - realism/**
      - tests/harness/**
      - tools/contracts/**
      - contracts/schemas/**
      - mk/mod-ntp.mk
      - .github/workflows/mod-ntp.yml
  pull_request:
    paths:
      - modules/ntp/**
      - engine/**
      - realism/**
      - tests/harness/**
      - tools/contracts/**
      - contracts/schemas/**
      - mk/mod-ntp.mk
      - .github/workflows/mod-ntp.yml

permissions:
  contents: read

env:
  GOWORK: "off"

jobs:
  module:
    runs-on: ubuntu-24.04
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"
      - name: Install nDPI and tshark, which the realism runner calls out of process
        run: |
          echo "wireshark-common wireshark-common/install-setuid boolean false" | sudo debconf-set-selections
          tools/ci/apt-install.sh --install-recommends libndpi-bin tshark
      - run: python -m pip install -r tools/contracts/requirements.txt
      - run: make lint-mod-ntp PYTHON=python
      - name: Score the committed fixture
        run: |
          python -m realism score --pcap modules/ntp/fixtures/ntp.pcapng \
            --labels modules/ntp/fixtures/expected.yaml --parsers tshark --output report.json
          python -c 'import json; r = json.load(open("report.json")); print(json.dumps([c.get("applications", c["check"]) for c in r["checks"]], indent=2))'

  harness:
    runs-on: ubuntu-24.04
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Install the harness and realism tools
        run: |
          echo "wireshark-common wireshark-common/install-setuid boolean false" | sudo debconf-set-selections
          tools/ci/apt-install.sh iproute2 tcpdump ethtool libndpi-bin tshark python3-yaml
      - run: cargo build -p fps-module-ntp --example fixture_peer
      - name: Record and score a fresh capture in the namespace harness
        run: |
          sudo env FPS_FIXTURE_PEER="$PWD/target/debug/examples/fixture_peer" \
            modules/ntp/fixtures/capture.sh "$RUNNER_TEMP/fixture"
          python3 -c 'import json, sys; r = json.load(open(sys.argv[1])); print(json.dumps([c.get("applications", c["check"]) for c in r["checks"]], indent=2))' \
            "$RUNNER_TEMP/fixture/ntp.report.json"
```

- File `mk/mod-ntp.mk` (coordinator file). Create it with exactly this content. We ran `make lint-mod-ntp test-mod-ntp` with it, and both passed, and then removed the file from our branch.

```make
# SPDX-License-Identifier: GPL-3.0-only
#
# MOD ntp module under modules/ntp/. The Rust tests also run in the rust
# job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_NTP_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-ntp

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds only a link to it.
LINT_TARGETS += lint-mod-ntp
lint-mod-ntp:
	mkdir -p $(MOD_NTP_OUT)/modules
	ln -sfn $(CURDIR)/modules/ntp $(MOD_NTP_OUT)/modules/ntp
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_NTP_OUT)/modules --tests --no-openapi

# The interpreter, catalog, and golden transcript tests, then the realism
# score of the committed fixture, which needs ndpiReader and tshark.
TEST_TARGETS += test-mod-ntp
test-mod-ntp:
	$(CARGO) test -p fps-module-ntp
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_NTP_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/ntp/fixtures/ntp.pcapng --labels modules/ntp/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/ntp/fixtures/ntp.pcapng \
	    --labels modules/ntp/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_NTP_OUT)/fixture.report.json; \
	else \
	  echo "test-mod-ntp skipped the fixture score because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness and scores it. It needs root,
# so make test leaves it out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-ntp
e2e-mod-ntp:
	modules/ntp/fixtures/capture.sh $(MOD_NTP_OUT)/capture

.PHONY: lint-mod-ntp test-mod-ntp e2e-mod-ntp
```

- Agent registry (AGT follow up). In `engine/fps-agent/src/modules.rs`, add this line to `REGISTRY` after `stateless!(fps_module_smb2, Smb2),`, and in `engine/fps-agent/Cargo.toml`, add the dependency below next to the other `fps-module-*` lines. `Ntp` is a unit struct that implements `Clone`, `Copy`, `Default`, and `TemplateFn`, so the `stateless!` macro fits, and `fps_module_ntp::FUNCTIONS` and `MODULE_NAME` exist for the catalog comparison. The agent tests that require a manifest for every registered module need `modules/ntp/manifest.yaml`, which this package ships.

```rust
    stateless!(fps_module_ntp, Ntp),
```

```toml
fps-module-ntp = { path = "../../modules/ntp/native" }
```

- [Agent](Agent.md): the sentence that lists the registry (`http1`, `dns`, `content`, `smtp`, `imap`, `pop3`, `ftp`, `smb2`, `http2`, and `ssh`) gains `ntp` when the registry line lands. The sentence about the clock is already true, because `now` reads the clock of the agent that renders the template.
- [Testing](Testing.md): the target table could gain the rows `lint-mod-ntp`, `test-mod-ntp`, and `e2e-mod-ntp`. The last one needs root.
- [Home](Home.md), [Implementation Plan](Implementation-Plan.md), [Swarm Protocol](Swarm-Protocol.md), [Coding Standards](Coding-Standards.md), and the sidebar: none.
- `Cargo.lock` (shared): this branch commits the entry for the new crate `fps-module-ntp`, which the workspace glob `modules/*/native` needs. A merge conflict in the lock file resolves by running `cargo metadata` on the merged tree.
