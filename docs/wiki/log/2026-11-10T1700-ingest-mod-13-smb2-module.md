## [2026-11-10] ingest | MOD-13 SMB2 module

We added the manifest, three scenarios, and a fixture to the `smb2` module under `modules/smb2/`. The scenarios `browse`, `download`, and `upload` share one session setup (NEGOTIATE, a synthetic NTLMv2 SESSION_SETUP exchange, TREE_CONNECT) and one teardown. `browse` lists the root directory and a subdirectory with QUERY_DIRECTORY, `download` reads a file in READ requests, and `upload` creates a file and writes it in WRITE requests. The sizes of the file and of the requests are variables. A flow has no arithmetic, so the length of the last request is a variable of its own, `tail_size` or `upload_tail`, and the README gives the rule that ties it to the file size. The evidence is `docs/handoff/MOD-13.md`. `native/tests/scenarios.rs` runs both sides of every scenario through the interpreter and checks the commands, the ids, the NTLM fields, the listings, and the sizes of every READ and WRITE for four download sizes and three upload sizes, for SMB 3.1.1, and for the parameter schema of the manifest. Golden transcripts hold the bytes of the three scenarios. `fixtures/smb2.pcapng` holds 18 sessions that `capture.sh` records in the QA-01 harness with a source port of its own per session (MOD-21). nDPI 4.2.0 classifies 18 of 18 as `NetBIOS.SMBv23` (ratio 1.0), and tshark 4.2.2 reports no parser error. We described the module in [Modules](Modules.md#the-smb2-module) and in `modules/smb2/README.md`.

### Shared page notes

- nDPI and the transport header: the 4 octet header of direct TCP is a zero octet and a 3 octet length, which is also the NetBIOS session header of a session message. nDPI needs nothing else, and it classifies SMB only on port 445: the same scenarios on port 4450 are `Unknown` in nDPI 4.2.0. nDPI 4.2.0 names the protocol `NetBIOS.SMBv23`, so `fixtures/expected.yaml` accepts `SMBv23` and `SMB`, and the manifest lists both. We could not run nDPI 4.14, which the CI realism runner uses, because the sandbox has 4.2.0 and the nDPI source was out of reach. The `module` job below is the first measurement. If 4.14 names the protocol otherwise, the name in `expected.yaml` and in the manifest changes, nothing else.
- Zeek: the catalog entry asks for no Zeek check, and Zeek is not installed in the sandbox. The module job scores the capture with `tshark,zeek`, so it also shows whether Zeek's SMB analyzer reports a parser error on the synthetic NTLM exchange. If it does, `FPS_REALISM_PARSERS` and the `--parsers` argument of the job drop `zeek` and the README says why.
- The manifest validator (`tools/contracts/validate.py`) limits a function summary to 200 characters, and thirteen summaries of the MOD-12 catalog exceeded it. We shortened them in `native/src/catalog.rs`, moved the list of body fields into the README, and regenerated the `functions` list of the manifest with `print_catalog`. A `set` value must be a string, so a counter starts as `"0"`.
- Cargo.lock: the crate gains the dev dependency `fps-backend-kernel` for the `fixture_peer` example, which is in the lock already, so only the dependency list of `fps-module-smb2` changes.
- `mk/mod-smb2.mk`: replace the file with the following. `lint-mod-smb2` validates the manifest and the scenarios through a directory of links, as `mod-imap.mk` does, but the module needs no link to the content module. `test-mod-smb2` scores the committed fixture when ndpiReader and tshark are installed. `e2e-mod-smb2` records a fresh capture.

  ```make
  # SPDX-License-Identifier: GPL-3.0-only
  #
  # MOD smb2 module under modules/smb2/. The Rust tests also run in the rust
  # job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

  # Scratch files go under the Cargo target directory, which Git ignores.
  MOD_SMB2_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-smb2

  # validate.py treats each directory under --modules as a module, so the target
  # validates the module through a parent directory that holds only a link.
  LINT_TARGETS += lint-mod-smb2
  lint-mod-smb2:
  	mkdir -p $(MOD_SMB2_OUT)/modules
  	ln -sfn $(CURDIR)/modules/smb2 $(MOD_SMB2_OUT)/modules/smb2
  	$(PYTHON) tools/contracts/validate.py --modules $(MOD_SMB2_OUT)/modules --tests --no-openapi

  # The tests of the native crate, then the realism score of the committed
  # fixture, which needs ndpiReader and tshark. The crate tests skip the tshark
  # parts without tshark.
  TEST_TARGETS += test-mod-smb2
  test-mod-smb2:
  	$(CARGO) test -p fps-module-smb2
  	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
  	  mkdir -p $(MOD_SMB2_OUT); \
  	  echo "$(PYTHON) -m realism score --pcap modules/smb2/fixtures/smb2.pcapng --labels modules/smb2/fixtures/expected.yaml --parsers tshark"; \
  	  $(PYTHON) -m realism score --pcap modules/smb2/fixtures/smb2.pcapng \
  	    --labels modules/smb2/fixtures/expected.yaml --parsers tshark \
  	    --output $(MOD_SMB2_OUT)/fixture.report.json; \
  	else \
  	  echo "test-mod-smb2 skipped the fixture score because ndpiReader or tshark is not installed"; \
  	fi

  # The 60 s fuzz test of the message readers and the template functions. It is
  # ignored in the normal test run.
  fuzz-mod-smb2:
  	$(CARGO) test -p fps-module-smb2 --release --test fuzz -- --ignored fuzz_60s --nocapture

  # Records a fresh capture in the QA-01 harness and scores it. It needs root,
  # so make test leaves it out, and it never overwrites the committed fixture.
  E2E_TARGETS += e2e-mod-smb2
  e2e-mod-smb2:
  	modules/smb2/fixtures/capture.sh $(MOD_SMB2_OUT)/capture

  .PHONY: lint-mod-smb2 test-mod-smb2 fuzz-mod-smb2 e2e-mod-smb2
  ```

- `.github/workflows/mod-smb2.yml`: add a `module` job, the path filters it needs, and the `GOWORK` variable that the other module workflows set. In `on.push.paths` and `on.pull_request.paths` add `realism/**`, `tools/contracts/**`, and `contracts/schemas/**`. Add under `permissions`:

  ```yaml
  env:
    GOWORK: "off"
  ```

  and add this job beside `tshark` and `fuzz`:

  ```yaml
    module:
      runs-on: ubuntu-24.04
      timeout-minutes: 25
      steps:
        - uses: actions/checkout@v4
        - uses: actions/setup-python@v5
          with:
            python-version: "3.12"
        - name: Install nDPI, tshark, and Zeek, which the checks call out of process
          run: realism/ci/install-tools.sh
        - run: python -m pip install -r tools/contracts/requirements.txt
        - run: make lint-mod-smb2 PYTHON=python
        - name: Score the committed fixture
          run: |
            python -m realism score --pcap modules/smb2/fixtures/smb2.pcapng \
              --labels modules/smb2/fixtures/expected.yaml --parsers tshark,zeek --output report.json
            python -c 'import json; r = json.load(open("report.json")); print(json.dumps([c.get("applications", c["check"]) for c in r["checks"]], indent=2))'
  ```

  The `tshark` job of the workflow keeps `FPS_REQUIRE_CAPTURE_TOOLS=1` and runs `--test tshark` only. The new test `fixture` also needs ndpiReader and PyYAML, so it skips itself there, and the `module` job covers the same score with Zeek.
- Agent registry: AGT-19 adds `fps-module-smb2` to `engine/fps-agent`, as the log entry of MOD-12 describes. The scenarios call only `smb2.*` functions and the built-ins `fill` and `rand_int`, so they need no other module.
- [Testing](Testing.md): the realism score of a module fixture is `python3 -m realism score --pcap FIXTURE --labels expected.yaml --parsers tshark`, and `native/tests/fixture.rs` of this module runs it from `cargo test` with the same skip rule as the tshark test (`FPS_REQUIRE_CAPTURE_TOOLS=1` makes a missing tool a failure).
- Worker brief: the sandbox has tshark 4.2.2, ndpiReader 4.2.0, tcpdump, and root, so the namespace harness and `capture.sh` ran here. Zeek and `cargo deny` are not installed.
