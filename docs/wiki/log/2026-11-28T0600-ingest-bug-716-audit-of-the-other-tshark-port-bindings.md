## [2026-11-28] ingest | bug #716: the other tshark -d bindings of realism are not exposed to a registered client port

- After #712, every `tshark -d` use under `realism/` was checked with Wireshark 4.2.2 by moving a session of the smtp, imap, http1 and http2 fixtures to client port 40000 (SAP NI). `decrypted.py` decodes TLS ports as TLS, which accepts any ClientHello, and STARTTLS ports as smtp or imap, which accept the clear text start and upgrade, so nothing reaches the client port. `messages.py` decodes the server port as the protocol of a plain text view, and the counts stay the same. No code changed; `test_a_client_port_that_wireshark_registers_loses_no_starttls_session` guards the choice. `classifier.py` `-d` is an ndpiReader flag, not tshark.

### Shared page notes

none
