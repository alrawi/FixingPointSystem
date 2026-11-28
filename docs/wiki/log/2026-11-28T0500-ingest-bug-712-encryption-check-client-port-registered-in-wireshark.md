## [2026-11-28] ingest | bug #712: encryption check no longer loses a session to a protocol registered on its client port

- The encryption check ran tshark with `-d tcp.port==<server port>,tls`. A STARTTLS session starts in clear text, TLS rejected it, and tshark gave it to the dissector of the client port. Port 40000 is SAP NI in Wireshark, so a sampled session on that port had no ClientHello (7 of 8 decrypted on PR #709). `TsharkTlsProbe.observe` now also decodes the client ports of the sampled sessions as TLS. A test moves a fixture session to port 40000.

### Shared page notes

Module Development, "Watch out for", add: "**A capture check loses a session on one client port in a run.** tshark decodes a port with `-d` and offers a segment that the first dissector rejects to the dissector of the other port of the pair, and Wireshark registers protocols on ephemeral ports (40000 is SAP NI). A STARTTLS session, whose clear text start TLS rejects, then showed no ClientHello and the encryption check decrypted 7 of 8 sessions only in runs that used that port. The check now decodes the client ports of the sampled sessions as TLS as well. If you write a tshark based check, bind both ports of a session or list the registered ports with `tshark -G decodes` (bug 712)."
