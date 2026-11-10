## [2026-11-10] ingest | MOD-12 SMB2 encoding functions

We added the native crate `fps-module-smb2` under `modules/smb2/native/`. It has thirty seven template functions. Ten encoders write the SMB2 requests NEGOTIATE, SESSION_SETUP, TREE_CONNECT, CREATE, READ, WRITE, CLOSE, QUERY_DIRECTORY, TREE_DISCONNECT, and LOGOFF, and eleven encoders write their responses and the error response. Five NTLM functions (`ntlm_negotiate`, `ntlm_challenge`, `ntlm_authenticate`, `spnego_accept`, `ntlm_session_key`) write a synthetic NTLM exchange with a full NTLMv2 response inside SPNEGO tokens, and `sign` signs a message with HMAC-SHA256. Six readers (`message_length`, `header_field`, `body_field`, `security_blob`, `ntlm_field`, `data`) read the messages of the peer, and `hex`, `unhex`, `chunks`, and `add` let a flow keep a token in a variable and count its requests. The evidence is `docs/handoff/MOD-12.md`. tshark 4.2.2 parses every message of four complete sessions (SMB 2.0.2, 2.1, 3.0.2, and 3.1.1, with reads of up to 200000 octets that span TCP segments) with no malformed packet and no expert warning or error. The NTLM functions match the NT hash, NTOWFv2, proof, and session base key of the MS-NLMP section 4.2 vectors. A golden test holds the bytes of two sessions, a flow test runs both sides of a scenario through the interpreter, and a seeded fuzz loop ran 280000 rounds in 60 seconds without a panic. We described the module in [Modules](Modules.md#the-smb2-module).

### Shared page notes

- [Modules](Modules.md): the page has a table row and a section for `smb2`, written in this package. The row says that the module has no scenarios and no fixture yet, because `modules/smb2/manifest.yaml` belongs to MOD-13.
- Agent registry: `engine/fps-agent/src/modules.rs` needs a dependency `fps-module-smb2 = { path = "../../modules/smb2/native" }` in `engine/fps-agent/Cargo.toml` and the entry `stateless!(fps_module_smb2, Smb2),` in `REGISTRY`, because `Smb2` is a unit value without state. The test `the_module_value_is_clone_send_and_sync` shows that the type satisfies the bounds. The agent test that lists the registered modules needs the new name.
- Cargo.lock: the file gains `fps-module-smb2` and three RustCrypto crates, `md4` 0.10.2 (NT hash), `md-5` 0.10.6, and `hmac` 0.12.1 (NTLMv2), all `MIT OR Apache-2.0`. The `sha2` crate is in the lock already. We could not run `cargo deny check` here, because the tool is not installed in the sandbox, so the `licenses` job of CI is the check.
- Module build files: `mk/mod-smb2.mk` and the workflow `.github/workflows/mod-smb2.yml` are not part of this package. The tshark test skips itself in the rust job of `ci.yml`, because that job has no tshark, so the acceptance item "tshark parses every encoded message" needs a job that installs it. The exact content of the two files follows. `lint-mod-smb2` and the realism steps belong to MOD-13, which adds the manifest.

  ```make
  # SPDX-License-Identifier: GPL-3.0-only
  #
  # MOD smb2 module under modules/smb2/. The Rust tests also run in the rust
  # job of ci.yml through cargo test --workspace, and lint-rust covers the crate.

  # The tests of the native crate. The tshark test skips itself without tshark.
  TEST_TARGETS += test-mod-smb2
  test-mod-smb2:
  	$(CARGO) test -p fps-module-smb2

  # The 60 s fuzz test of the message readers and the template functions. It is
  # ignored in the normal test run.
  fuzz-mod-smb2:
  	$(CARGO) test -p fps-module-smb2 --release --test fuzz -- --ignored fuzz_60s --nocapture

  .PHONY: test-mod-smb2 fuzz-mod-smb2
  ```

  ```yaml
  # SPDX-License-Identifier: GPL-3.0-only
  #
  # Workflow of the smb2 module under modules/smb2/. The rust job of ci.yml runs
  # the Rust tests of the native crate, but there the tshark test skips itself.
  # The tshark job installs tshark and runs it with FPS_REQUIRE_CAPTURE_TOOLS=1,
  # so tshark must parse every message that the functions encode (MOD-12
  # acceptance). The fuzz job runs the 60 s fuzz test.
  name: mod-smb2

  on:
    push:
      branches: [main]
      paths:
        - modules/smb2/**
        - engine/fps-flow/**
        - Cargo.lock
        - mk/mod-smb2.mk
        - .github/workflows/mod-smb2.yml
    pull_request:
      paths:
        - modules/smb2/**
        - engine/fps-flow/**
        - Cargo.lock
        - mk/mod-smb2.mk
        - .github/workflows/mod-smb2.yml

  permissions:
    contents: read

  jobs:
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
            sudo apt-get update -q
            sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends tshark
        - name: Parse every encoded message with tshark
          env:
            FPS_REQUIRE_CAPTURE_TOOLS: "1"
          run: cargo test -p fps-module-smb2 --test tshark -- --nocapture
    fuzz:
      runs-on: ubuntu-24.04
      timeout-minutes: 30
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - name: Fuzz the readers and the functions for 60 seconds
          run: cargo test -p fps-module-smb2 --release --test fuzz -- --ignored fuzz_60s --nocapture
  ```

- MOD-13 (manifest and scenarios): `cargo run -p fps-module-smb2 --example print_catalog` prints the `functions:` list that the manifest must hold, and `tests/catalog.rs` compares `modules/smb2/manifest.yaml` with the native catalog as soon as the file exists. A scenario reads a message with two `expect` actions, the 4 octet transport header and then `smb2.message_length(last)` octets, and keeps the message in `last`, because `set` renders to lossy text. `tests/data/smb2_flow.yaml` shows the idioms for the session setup exchange, for a token that travels in a variable as `smb2.hex(...)` and `smb2.unhex(vars.name)`, and for a loop that numbers its messages with `smb2.add(vars.mid)`.
- MOD-13 limits that follow from the engine: a flow has no arithmetic and no loop counter, so the crate has `add` and `chunks`. A server answers a read with as many octets as the request names, so a file size that is not a multiple of the read size needs a size variable for the last chunk. The module writes no compound requests, create contexts, leases, or SMB 3.x signing and encryption. nDPI 4.2.0 must classify the sessions as SMB. We did not measure that, because this package has no capture, so MOD-13 must check how nDPI treats the NetBIOS session header, and the transport header of every message of this crate is the 4 octet header of direct TCP that nDPI sees on port 445.
- [Testing](Testing.md): the page could name the pattern that this package uses for a protocol without a capture of its own. The functions are acceptance tested by writing the encoded messages into a pcap file in the test and running tshark over it, with `FPS_REQUIRE_CAPTURE_TOOLS=1` in the CI job that installs tshark.
- Worker brief: the brief says that the sandbox has no tshark, but tshark 4.2.2 is installed here at `/usr/bin/tshark`, so the tshark test ran locally.
