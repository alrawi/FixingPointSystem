## [2026-11-17] ingest | bug-618 The message count check attributes messages per connection

The `messages` check of the realism runner no longer adds the messages of two connections that reuse a client port (issue #618). `realism/messages.py` took the tshark counts per four tuple and looked them up by `session.key`, which two connections on a reused client port share, so each session got the sum (14 HTTP/2 requests for two sessions of 7). The view of `realism/decrypted.py` was already right since RLM-24. `TsharkMessageProbe` now asks tshark for `frame.number` and reports the messages of each frame (`MessageObservation.frames`), and `messages.counts_by_session` replays the session rule of `SessionResolver` over the frames of the capture to give each frame to its connection. A probe that reports no frames, such as the stub, still looks up the four tuple.

Evidence: `test_the_messages_of_sessions_on_a_reused_client_port_count_per_session` in `realism/tests/test_decrypted.py` writes a view of two sessions on one client port with 7 HTTP/2 requests each and runs the check with tshark. Before the change it failed with `surplus http2 request: found 14, expected 7`, after it each session reports 7. `test_parse_frame_fields_keeps_the_frame_of_every_count` covers the new parser. `python3 -m pytest realism` passes.

Findings:

- The tshark findings of the parser error check, the nDPI verdict and the TLS hello records are still keyed by four tuple (see the RLM-24 entry). Only the message count is per connection now.

Shared page notes:

- [Module Development](Module-Development.md), section "Watch out for", entry "The decrypted view of two sessions on one client port counts the messages twice.": replace its text with the following. The old text says the view fix alone cured the symptom, but the count check still merged the two connections until bug 618.

  > **Two sessions on one client port counted their messages twice.** You saw a message count of 14 in a session that should hold 7 in the `messages` check, while the capture has two connections on one client port, so the `module (http2)` job failed on a correct capture whenever the kernel reused an ephemeral port. Two things joined the sessions: a view that gave every session the same initial sequence numbers (a repeated SYN to a dissector), and a count that tshark reported per four tuple and the check looked up by four tuple. The realism runner now gives every session of a decrypted view its own initial sequence numbers (`realism.decrypted.initial_sequence_numbers`, RLM-24), starts a new session on a SYN after a FIN or an RST, and counts messages per frame and attributes each frame to its connection (bug 618). A capture that reuses a client port therefore passes with the right count per session; if you write a view or a capture by hand, give each connection its own initial sequence number and close a connection with a FIN or an RST before the next one starts on the same ports.

- [Testing](Testing.md), [Home](Home.md), [Implementation Plan](Implementation-Plan.md), [Swarm Protocol](Swarm-Protocol.md), [Coding Standards](Coding-Standards.md): none.
