## [2026-11-13] ingest | MOD-30 STARTTLS ports in the smtp and imap capture scripts

Issue #546. RLM-21 added `--starttls-port PORT:PROTOCOL` to `python3 -m realism decrypted` and set it in `mod-smtp.yml` and `mod-imap.yml`. The capture scripts and fixture tests of the two modules still called the command without it and relied on the TLS heuristic of tshark. They now pass `--starttls-port 5587:smtp` (`modules/smtp/fixtures/capture.sh`, `modules/smtp/tests/fixture.rs`) and `--starttls-port 1143:imap` (`modules/imap/fixtures/capture.sh`, `modules/imap/tests/fixture.rs`). The scripts build the argument from their `starttls_port` variable, so a changed fixture port changes both the scenario port and the option. The labels keep naming the view ports 587 and 143.

Evidence, local, with tshark 4.2.2 and nDPI 4.2.0 (no Zeek): `capture.sh` of both modules ran in the namespace harness with 4 sessions per scenario and scored the decrypted view with the option (smtp: 4 of 4 TLS sessions decrypt, parsers 0 errors, passed; imap: the same). `python3 -m realism decrypted --starttls-port ...` on the committed captures gives 8 of 8 and passed for both. `cargo test -p fps-module-smtp --test fixture` and `-p fps-module-imap --test fixture` pass with `FPS_REQUIRE_CAPTURE_TOOLS=1`. The Zeek parts of the scripts did not run, because Zeek is not installed here.

The module READMEs and the "Watch out for" entry on STARTTLS ports in `docs/wiki/Module-Development.md` name the option in the first party scripts. The text on the option itself was already in the guide (coordinator, from the RLM-21 notes).

### Shared page notes

- Module Development: done in this PR (MOD workstream), nothing to apply.
- [Testing](Testing.md), [Implementation Plan](Implementation-Plan.md), [Home](Home.md): none.
