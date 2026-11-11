## [2026-11-11] ingest | MOD-17 LDAP module

We added the `ldap` module under `modules/ldap`. Its crate `fps-module-ldap` implements the LDAPv3 messages for bind, search, and unbind as a native extension (decision D10), with a BER codec written in the crate, the search filters of RFC 4515, an LDIF reader, and a server side that answers binds and searches from a fixture directory. The four scenarios are `bind_search`, `failed_bind`, `directory_browse`, and `rootdse_probe`. The evidence is `docs/handoff/MOD-17.md`. `fixtures/ldap.pcapng` holds 24 sessions that `capture.sh` records in the QA-01 harness with a source port of its own per session (MOD-21). nDPI 4.2.0 classifies 24 of 24 as `LDAP` (ratio 1.0) and tshark 4.2.2 reports no parser error. We described the module in [Modules](Modules.md#the-ldap-module) and in `modules/ldap/README.md`.

The library finding: we wrote the BER codec instead of adding a crate. LDAP needs a few definite length forms, integers, booleans, octet strings, and constructed values, and the crate needs a reader that takes one message at a time when the interpreter asks for it. The crate has no dependency besides `fps-flow` and `rand`, so `cargo deny` has nothing new to check.

### Shared page notes

- nDPI and the first message: nDPI 4.2.0 labels a session MQTT when the first bind request carries a name, with or without a password (bind requests of 20 to 64 octets, measured with synthetic captures), and LDAP when the first message is the 14 octet anonymous bind. A session that continues with named binds after the anonymous bind stays LDAP. Every scenario therefore opens with an anonymous bind, as many clients do to test a connection. The sandbox has nDPI 4.2.0 only. CI uses nDPI 4.14, which the sandbox could not fetch, so the `module` job below is the first measurement with 4.14.
- Zeek: the catalog entry asks for no Zeek check, and the module has none. Zeek is not installed in the sandbox.
- Cargo.lock: the lock file gains the entry of the new crate `fps-module-ldap`, which depends on workspace crates and on `rand`, `rand_pcg`, `serde_norway`, and `tokio`, all in the lock already. The entry is part of the pull request. `cargo deny` was not run here.
- `mk/mod-ldap.mk` (new file, a coordinator file):

  ```make
  # SPDX-License-Identifier: GPL-3.0-only
  #
  # MOD ldap module under modules/ldap/. The Rust tests also run in the rust
  # job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

  # Scratch files go under the Cargo target directory, which Git ignores.
  MOD_LDAP_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-ldap

  # validate.py treats each directory under --modules as a module, so the target
  # validates the module through a parent directory that holds only a link. The
  # module requires no other module.
  LINT_TARGETS += lint-mod-ldap
  lint-mod-ldap:
  	mkdir -p $(MOD_LDAP_OUT)/modules
  	ln -sfn $(CURDIR)/modules/ldap $(MOD_LDAP_OUT)/modules/ldap
  	$(PYTHON) tools/contracts/validate.py --modules $(MOD_LDAP_OUT)/modules --tests --no-openapi

  # The interpreter, catalog, golden transcript, and tshark tests, then the
  # realism score of the committed fixture, which needs ndpiReader and tshark.
  TEST_TARGETS += test-mod-ldap
  test-mod-ldap:
  	$(CARGO) test -p fps-module-ldap
  	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
  	  mkdir -p $(MOD_LDAP_OUT); \
  	  echo "$(PYTHON) -m realism score --pcap modules/ldap/fixtures/ldap.pcapng --labels modules/ldap/fixtures/expected.yaml --parsers tshark"; \
  	  $(PYTHON) -m realism score --pcap modules/ldap/fixtures/ldap.pcapng \
  	    --labels modules/ldap/fixtures/expected.yaml --parsers tshark \
  	    --output $(MOD_LDAP_OUT)/fixture.report.json; \
  	else \
  	  echo "test-mod-ldap skipped the fixture score because ndpiReader or tshark is not installed"; \
  	fi

  # Records a fresh capture in the QA-01 harness and scores it. It needs root,
  # so make test leaves it out, and it never overwrites the committed fixture.
  E2E_TARGETS += e2e-mod-ldap
  e2e-mod-ldap:
  	modules/ldap/fixtures/capture.sh $(MOD_LDAP_OUT)/capture

  .PHONY: lint-mod-ldap test-mod-ldap e2e-mod-ldap
  ```

- `.github/workflows/mod-ldap.yml` (new file, a coordinator file):

  ```yaml
  # SPDX-License-Identifier: GPL-3.0-only
  #
  # Workflow of the ldap module under modules/ldap/. The rust job of ci.yml runs
  # the Rust tests of the native crate, but there the tshark test skips itself.
  # The tshark job installs tshark and runs it with FPS_REQUIRE_CAPTURE_TOOLS=1,
  # so tshark must parse every message that the functions encode. The module job
  # validates the manifest and scenarios and scores the committed fixture with
  # nDPI and tshark, so nDPI must classify at least 99% of its sessions as LDAP
  # and no parser reports an error (MOD-17 acceptance). The realism workflow
  # records a fresh capture of the module in the QA-01 namespace harness and
  # checks it the same way.
  name: mod-ldap

  on:
    push:
      branches: [main]
      paths:
        - modules/ldap/**
        - engine/fps-flow/**
        - Cargo.lock
        - realism/**
        - tools/contracts/**
        - contracts/schemas/**
        - mk/mod-ldap.mk
        - .github/workflows/mod-ldap.yml
    pull_request:
      paths:
        - modules/ldap/**
        - engine/fps-flow/**
        - Cargo.lock
        - realism/**
        - tools/contracts/**
        - contracts/schemas/**
        - mk/mod-ldap.mk
        - .github/workflows/mod-ldap.yml

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
        - run: make lint-mod-ldap PYTHON=python
        - name: Score the committed fixture
          run: |
            python -m realism score --pcap modules/ldap/fixtures/ldap.pcapng \
              --labels modules/ldap/fixtures/expected.yaml --parsers tshark --output report.json
            python -c 'import json; r = json.load(open("report.json")); print(json.dumps([c.get("applications", c["check"]) for c in r["checks"]], indent=2))'
    tshark:
      runs-on: ubuntu-24.04
      timeout-minutes: 30
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - name: Install tshark
          run: |
            echo "wireshark-common wireshark-common/install-setuid boolean false" | sudo debconf-set-selections
            tools/ci/apt-install.sh tshark
        - name: Parse every encoded message with tshark
          env:
            FPS_REQUIRE_CAPTURE_TOOLS: "1"
          run: cargo test -p fps-module-ldap --test tshark -- --nocapture
  ```

- Agent registry: add `fps-module-ldap = { path = "../../modules/ldap/native" }` to `engine/fps-agent/Cargo.toml`, add the name `"ldap"` to the module list in the test of `engine/fps-agent/src/modules.rs`, and add this entry to the registry after the `ssh` entry (AGT follow-up). `Ldap` holds the message that `recv` decoded and the search that `matches` ran, so it is a value with state, and every session clones its own copy of the fresh value, as it does for `Http2`:

  ```rust
      Registration {
          name: fps_module_ldap::MODULE_NAME,
          catalog: || catalog_of!(fps_module_ldap),
          build: |ctx| {
              if ctx.given.is_some() {
                  return Err(BuildError::Settings(
                      "the plan gives settings, but its native functions take none".to_owned(),
                  ));
              }
              Ok(Box::new(fps_module_ldap::Ldap::default()))
          },
      },
  ```

- The manifest validator limits a function summary to 200 characters, and a `set` value must be a string. A nested call has the type `bytes`, which a `text` parameter does not accept, so the scenarios keep a filter in a variable with a `set` action first.
- Worker brief: the sandbox has tshark 4.2.2, ndpiReader 4.2.0, tcpdump, and root, so the namespace harness and `capture.sh` ran here. Zeek, `cargo deny`, and nDPI 4.14 are not available.
