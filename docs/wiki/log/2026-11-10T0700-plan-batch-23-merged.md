## [2026-11-10] plan | Batch 23 merged

The coordinator merged CAP-03 in PR #365, UI-04 in PR #368, ENG-24 in PR #366, AGT-12 in PR #369, MOD-20 in PR #370, MOD-21 in PR #367, RLM-07 in PR #363, and CON-19 in PR #364. Several CI jobs hung for one to two hours in the step that installs nDPI, tshark, and Zeek (an apt mirror problem), so the coordinator merged where the jobs that test each change had passed and only unrelated module installs were pending, and said so in each review. MOD-06 (PR #371) stays open: the realism job installs nDPI 4.2.0, which has no HTTP/2 protocol, so RLM-12 brings a current nDPI first.

With AGT-12 a capture starts, stops, and downloads through the REST API against the real agent over mutual TLS, and AGT-14's TLS key block lands in the file when export_tls_keys is set. We applied the shared page notes to [Controller](../Controller.md) and [Home](../Home.md), and the mod-smtp and mod-imap workflows run the decrypted STARTTLS checks of MOD-20. The catalog gains RLM-12 (current nDPI, install timeout, shared decrypted view), TRN-15 (TLS plaintext backpressure, mail and h2 profiles), ENG-25 (int_bytes and now), CTL-38 (distributions in the compiler, scenario counts in the report), and CON-20 (stream events, metric source label, provenance fields, which needs owner approval). AGT-16 now also switches the agent to per session connectors and offers the TRN-15 profiles.

### Shared page notes

None.
