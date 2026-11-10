## [2026-11-10] ingest | MOD-20 STARTTLS scenarios for smtp and imap

We added the scenario `starttls_submission` to the smtp module and `starttls_mailbox` to the imap module, both on the `tls_upgrade` action of ENG-20. The smtp client uses `firefox-139` on TLS 1.3 against `server-tls13`, and the imap client uses `safari-18.5` on TLS 1.2 against `server-tls12`. `smtp.ehlo_reply` gained the parameter `starttls`. Both modules have a TLS-aware test harness (`tests/common/mod.rs` and `tests/common/tls.rs`), STARTTLS tests, golden transcripts, and a fixture that `capture.sh` records with the exported keys. The client peer writes an NSS key log, `capture.sh` injects it with `editcap --inject-secrets`, the labels name the profile and its JA4, and the encryption check of the realism runner decrypts all 8 STARTTLS sessions of each fixture and matches every JA4. The fixtures use the ports 5587 (smtp) and 1143 (imap) for the STARTTLS sessions, because the labels tell sessions apart by port and the ports 587 and 143 carry the plain sessions.

nDPI 4.2.0 labels the smtp STARTTLS sessions `SMTP` (it settles at the greeting and keeps the label through the handshake) and the imap STARTTLS sessions `IMAPS`. The labels accept `SMTP` and `SMTPS` for the first and `IMAPS` for the second. Zeek 9.0.1 cannot decrypt TLS, so each module has `fixtures/decrypted_check.py`. It decrypts the capture with tshark, checks the dialogue inside TLS, builds a decrypted view of plain sessions, scores the view with nDPI (`SMTP` and `IMAP`, ratio 1.0), and reads the view with Zeek when `--zeek` is given. For smtp that is the check of `zeek_check.py`, for imap a conservative check of `conn.log` and `weird.log`, because Zeek has no IMAP analyzer. Zeek was not installed in the sandbox that wrote the package, so the Zeek part of the view check is proven by the CI run only. The same package fixed the direction of the `follow,tls` stream in the check, which now reads it from the header of each stream and no longer assumes that node 0 is the server.

### Shared page notes

- Workflow `.github/workflows/mod-smtp.yml`, in the job `module`, after the step "Check smtp.log, files.log, and weird.log of the committed fixture" (the step names the committed fixture, which now holds 56 sessions, 64 messages in the plain ones, so the counts stay right):

  ```yaml
      - name: Decrypt the STARTTLS sessions of the committed fixture and read the decrypted view with Zeek
        run: python modules/smtp/fixtures/decrypted_check.py modules/smtp/fixtures/smtp.pcapng --sessions 8 --zeek /opt/zeek/bin/zeek
  ```

- Workflow `.github/workflows/mod-imap.yml`, in the job `module`, after the step "Score the committed fixture":

  ```yaml
      - name: Test the decrypted check
        run: PYTHONDONTWRITEBYTECODE=1 python -m unittest discover -s modules/imap/fixtures
      - name: Decrypt the STARTTLS sessions of the committed fixture and read the decrypted view with Zeek
        run: python modules/imap/fixtures/decrypted_check.py modules/imap/fixtures/imap.pcapng --sessions 8 --zeek /opt/zeek/bin/zeek
  ```

  The jobs already install tshark 4.2.2 and Zeek through `realism/ci/install-tools.sh`, and `editcap` comes with `tshark`. The `realism` workflow needs no change, because `realism/ci/module-realism.sh` runs `capture.sh`, which now runs `decrypted_check.py` with Zeek when Zeek is installed.
- `mk/mod-smtp.mk`, target `test-mod-smtp`: after the `zeek_check.py` block add

  ```make
  	@zeek="$${FPS_ZEEK:-$$(command -v zeek || true)}"; \
  	if [ -z "$$zeek" ] && [ -x /opt/zeek/bin/zeek ]; then zeek=/opt/zeek/bin/zeek; fi; \
  	if command -v tshark >/dev/null 2>&1 && command -v ndpiReader >/dev/null 2>&1; then \
  	  $(PYTHON) modules/smtp/fixtures/decrypted_check.py modules/smtp/fixtures/smtp.pcapng --sessions 8 $${zeek:+--zeek "$$zeek"}; \
  	else \
  	  echo "test-mod-smtp skipped the decrypted check because tshark or ndpiReader is not installed"; \
  	fi
  ```

  The same lines go into `mk/mod-imap.mk` with `imap` for `smtp`, together with the line `PYTHONDONTWRITEBYTECODE=1 $(PYTHON) -m unittest discover -s modules/imap/fixtures` after `$(CARGO) test -p fps-module-imap`. The comment above `test-mod-imap` says that the module has no Zeek check, which stays true for the plain sessions. Both modules run the same check inside `cargo test` already (`tests/fixture.rs`), which skips itself when a tool is missing, so these lines are optional for coverage and needed only for the Zeek part.
- Agent (AGT-13 and AGT-14): the registry must dispatch `smtp`, `imap`, and `content` calls, and its TLS profile builder must offer `firefox-139`, `safari-18.5`, `server-tls13`, and `server-tls12` for `tls_upgrade`. The browser profiles offer the ALPN ids `h2` and `http/1.1`, which a mail client never sends, and the server profiles select none. The agent therefore needs a non-browser mail client profile without ALPN (for the plan to show the ClientHello of a mail client) and the server profiles without ALPN. We did not define such a profile here, because the TLS module owns the registry, and we used the browser profiles as the brief allows. The profiles of the two `tests/common/tls.rs` files are copies of the registry of the `http1` tests and go away when the agent offers a builder that the tests can call.
- [Realism](Realism.md) and RLM: the labels file already supports `tls_profile` and the `tls` section, so the fixtures needed no change of the runner. The check that Zeek or nDPI sees the messages inside TLS has no home in the runner. Both `decrypted_check.py` scripts are copies of one another except for the protocol. A shared RLM check, for example `python3 -m realism decrypted --pcap ... --view-port ... --protocol smtp|imap`, would replace the two copies and the one of MOD-04.
- [Testing](Testing.md): `tests/fixture.rs` of both modules honors `FPS_REQUIRE_CAPTURE_TOOLS=1`, as the tests of the TLS module do, and skips itself when `python3`, `tshark`, or `ndpiReader` is missing.
- [Modules](Modules.md): the rows and the sections of `smtp` and `imap` were updated in this package, which is the workstream component page.
