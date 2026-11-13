## [2026-11-13] ingest | QA-13, the Zeek event queue weird at the end of the Gate 2 mix

Issue #548. One qa-gate2 mix run of CTL-49 failed its parser error check on the Zeek weird `event_queue_overflow_drain` at +141.023 s of a capture of 141.023 s, with no session, and passed on rerun.

**Cause.** Zeek's `src/Event.cc` raises the weird in `EventMgr::OverflowDrain` when the event queue holds more than `max_event_queue_size` events (`const max_event_queue_size: count` in `src/const.bif`; 0 disables the limit). It builds the `net_weird` event by hand, so the weird has no connection and the weird log gives it no endpoints. The parser error check counts a weird without endpoints as unattributed. The mix capture ends many connections at once, so the queue can overflow while Zeek reads the last packets. This is read from the Zeek source and from the first failure; no worker had Zeek installed, so the new CI runs are the first test of the setting.

**Fix.** `tests/gates/gate2/zeek-gate2.sh` starts the real Zeek with `max_event_queue_size=0` when it reads a capture, and `mix.sh` sets `FPS_ZEEK` to it for the realism runner of the mix stage. No rule leaves a weird out, so every weird of a session still fails. `test_zeek_gate2.py` runs the wrapper against a stand in for Zeek and shows through the realism Zeek runner that the weird of a session stays a finding and the queue weird is gone.

### Shared page notes

- Module Development, Watch out for: not needed. The setting applies to the mix capture only and no module author meets it.
- Realism page and `realism/parser_errors.py` (RLM-23 holds these files in this batch, so QA-13 does not edit them): if the owner wants a second line of defense, add a named rule to the parser error check that does not count a Zeek weird of `event_queue_overflow_drain` that has no session (a finding with no complete 5-tuple), and list it under `not_counted` with the rule name, as the port collision rule does. A weird of a session with the same name would still count. The rule is not needed while `zeek-gate2.sh` is in place, and the suite has no copy of it.
- Testing page: nothing.
