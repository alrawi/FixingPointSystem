## [2026-11-10] ingest | MOD-11 ssh module

We added the `ssh` module under `modules/ssh`. Its crate `fps-module-ssh` implements a real SSH 2.0 transport as a native extension (decision D10): the identification lines, `SSH_MSG_KEXINIT`, a `curve25519-sha256` key exchange, an `ssh-ed25519` host key that the client verifies, the key derivation of RFC 4253, and `chacha20-poly1305@openssh.com` packets. There are four scenarios, `interactive_shell`, `remote_command`, `scp_upload`, and `scp_download`, and `commands`, `typed`, and `file_bytes` set the length of a session. `fixtures/capture.sh` recorded `fixtures/ssh.pcapng` in the QA-01 harness: 16 sessions with 16 different 5-tuples (one source port per session, the MOD-21 fix), nDPI 4.2.0 labels all 16 `SSH`, and tshark 4.2.2 reports no parser error. The tests run every scenario between two in-memory peers, flip a bit to prove that a damaged packet fails the session, and compare the bytes at seed 42 with golden files.

The library finding: the RustCrypto crates (`ssh-key`, `ssh-encoding`, `x25519-dalek`, `chacha20`, `poly1305`, `sha2`) are licensed Apache-2.0 or MIT, which GPL-3.0 accepts. A full SSH library such as `russh` runs its own async session loop and cannot be driven one packet at a time by the interpreter, so the module writes the packet layer and the key exchange on those primitives. The scenario language has no action that gives received bytes to a function, so each receive is an `expect` with the count `ssh.size(last)`, a second `expect`, and a `set` that calls `ssh.recv(last, ...)`. Zeek was not installed in the sandbox, so `zeek_check.py` is tested against synthetic logs and proven by the CI run.

### Shared page notes

- `mk/mod-ssh.mk` (new file, a coordinator file):

  ```make
  # SPDX-License-Identifier: GPL-3.0-only
  #
  # MOD ssh module under modules/ssh/. The Rust tests also run in the rust
  # job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

  # Scratch files go under the Cargo target directory, which Git ignores.
  MOD_SSH_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-ssh

  # validate.py treats each directory under --modules as a module, so the target
  # validates the module through a parent directory that holds only a link. The
  # module requires no other module.
  LINT_TARGETS += lint-mod-ssh
  lint-mod-ssh:
  	mkdir -p $(MOD_SSH_OUT)/modules
  	ln -sfn $(CURDIR)/modules/ssh $(MOD_SSH_OUT)/modules/ssh
  	$(PYTHON) tools/contracts/validate.py --modules $(MOD_SSH_OUT)/modules --tests --no-openapi

  # The interpreter, catalog, and golden transcript tests, the tests of the Zeek
  # check, then the realism score of the committed fixture, which needs
  # ndpiReader and tshark, and the Zeek check, which needs Zeek. The count is the
  # number of sessions that capture.sh records with its default of 4 sessions per
  # scenario.
  TEST_TARGETS += test-mod-ssh
  test-mod-ssh:
  	$(CARGO) test -p fps-module-ssh
  	PYTHONDONTWRITEBYTECODE=1 $(PYTHON) -m unittest discover -s modules/ssh/fixtures
  	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
  	  mkdir -p $(MOD_SSH_OUT); \
  	  echo "$(PYTHON) -m realism score --pcap modules/ssh/fixtures/ssh.pcapng --labels modules/ssh/fixtures/expected.yaml --parsers tshark"; \
  	  $(PYTHON) -m realism score --pcap modules/ssh/fixtures/ssh.pcapng \
  	    --labels modules/ssh/fixtures/expected.yaml --parsers tshark \
  	    --output $(MOD_SSH_OUT)/fixture.report.json; \
  	else \
  	  echo "test-mod-ssh skipped the fixture score because ndpiReader or tshark is not installed"; \
  	fi
  	@zeek="$${FPS_ZEEK:-$$(command -v zeek || true)}"; \
  	if [ -z "$$zeek" ] && [ -x /opt/zeek/bin/zeek ]; then zeek=/opt/zeek/bin/zeek; fi; \
  	if [ -n "$$zeek" ]; then \
  	  echo "modules/ssh/fixtures/zeek_check.py modules/ssh/fixtures/ssh.pcapng --sessions 16"; \
  	  $(PYTHON) modules/ssh/fixtures/zeek_check.py modules/ssh/fixtures/ssh.pcapng \
  	    --zeek "$$zeek" --sessions 16; \
  	else \
  	  echo "test-mod-ssh skipped the Zeek check because zeek is not installed"; \
  	fi

  # Records a fresh capture in the QA-01 harness, scores it, and runs the
  # session count check and the Zeek check. It needs root, so make test leaves it
  # out, and it never overwrites the committed fixture.
  E2E_TARGETS += e2e-mod-ssh
  e2e-mod-ssh:
  	modules/ssh/fixtures/capture.sh $(MOD_SSH_OUT)/capture

  .PHONY: lint-mod-ssh test-mod-ssh e2e-mod-ssh
  ```

  The recipe lines start with a tab. If the Makefile includes `mk/*.mk` by a wildcard, no other change is needed. If it lists them, add `mk/mod-ssh.mk`.
- `.github/workflows/mod-ssh.yml` (new file, a coordinator file):

  ```yaml
  # SPDX-License-Identifier: GPL-3.0-only
  #
  # Workflow of the ssh module under modules/ssh/. The rust job of ci.yml runs
  # its Rust tests. The module job validates the manifest and scenarios and
  # scores the committed fixture with nDPI, tshark, and Zeek, so nDPI must
  # classify at least 99% of its sessions as SSH, no parser reports an error, and
  # ssh.log holds one row for every session with the negotiated algorithms.
  # The realism workflow records a fresh capture of the module in the QA-01
  # namespace harness and checks it the same way. The job runs on a GitHub hosted
  # runner, never on a self hosted one.
  name: mod-ssh

  on:
    push:
      branches: [main]
      paths:
        - modules/ssh/**
        - realism/**
        - capture/**
        - tools/contracts/**
        - contracts/schemas/**
        - mk/mod-ssh.mk
        - .github/workflows/mod-ssh.yml
    pull_request:
      paths:
        - modules/ssh/**
        - realism/**
        - capture/**
        - tools/contracts/**
        - contracts/schemas/**
        - mk/mod-ssh.mk
        - .github/workflows/mod-ssh.yml

  permissions:
    contents: read

  env:
    GOWORK: "off"

  jobs:
    module:
      runs-on: ubuntu-24.04
      steps:
        - uses: actions/checkout@v4
        - uses: actions/setup-python@v5
          with:
            python-version: "3.12"
        - name: Install nDPI, tshark, and Zeek, which the checks call out of process
          run: realism/ci/install-tools.sh
        - run: python -m pip install -r tools/contracts/requirements.txt
        - run: make lint-mod-ssh PYTHON=python
        - name: Unit tests of the Zeek check
          run: PYTHONDONTWRITEBYTECODE=1 python -m unittest discover -s modules/ssh/fixtures
        - name: Score the committed fixture
          run: |
            python -m realism score --pcap modules/ssh/fixtures/ssh.pcapng \
              --labels modules/ssh/fixtures/expected.yaml --parsers tshark,zeek --output report.json
            python -c 'import json; r = json.load(open("report.json")); print(json.dumps([c.get("applications", c["check"]) for c in r["checks"]], indent=2))'
        - name: Check ssh.log of the committed fixture
          run: |
            python modules/ssh/fixtures/zeek_check.py modules/ssh/fixtures/ssh.pcapng --sessions 16
  ```

- Modules page rows: this package added the `ssh` row to the table and the section "The ssh module" to `docs/wiki/Modules.md` (the workstream component page), so the page needs no further edit.
- `Cargo.lock`: this package adds `ssh-key`, `ssh-encoding`, `x25519-dalek`, `chacha20`, `poly1305`, `signature`, and their dependencies. The lock file is part of the PR. `cargo deny` was not run here, so the CI job that runs it is the proof that the licenses pass.
- The realism job: `realism/ci/module-realism.sh` runs `capture.sh` of every touched module, and `capture.sh` of `ssh` needs only the tools that the other modules need.
- Agent (AGT-13 and AGT-14): the registry must dispatch `ssh` calls and build one `Ssh` per flow side, because the value holds the keys and the packet counters of one session. `fixture_peer` shows the pattern.
