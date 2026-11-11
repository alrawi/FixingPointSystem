## [2026-11-11] plan | Batch 28 merged

The coordinator merged:
- UI-09 in PR #444, AGT-20 in PR #445 and MOD-16 in PR #447.
- MOD-18 in PR #448, MOD-17 in PR #449 and MOD-15 in PR #455. After each merge the coordinator merged main into the next one: Cargo.lock regenerated with `cargo update -w`, and the module rows of the Modules page kept from both sides. The lock verifies with `--locked`, and each crate builds against main.
- RLM-17 in PR #446. On its branch the coordinator also:
  - merged main;
  - refreshed the controller's copy of the realism report schema (format 0.12.0);
  - added the now required `realism.check_failures` series to the controller's realism fixture, mapped by the generic rule until CTL-42.
- CON-24 in PR #443, which the owner approved on 2026-11-10. On its branch the coordinator also:
  - made the agent report the three drop causes, with `dropped` as their sum (`Done` carries them, and `cut_at_stop` is read from `Capture::stats` after the stop);
  - bumped `ENGINE_API` to 0.2.4 and the REST `APIVersion` to 0.11.0;
  - passed the causes through the controller's capture service and REST view, absent for older agents;
  - regenerated `agentpb`, `internal/api`, the UI types and the Python SDK;
  - added tests on both sides.

What the workers found:
- nDPI classifies NTP only on port 123.
- nDPI 4.2 labels an LDAP session MQTT when its first bind carries a name, so the LDAP scenarios open with an anonymous bind.
- HLS segment continuity counters must continue across segments, or tshark reports missing frames.
- A late datagram of a finished SIP call opens a failing session on the UDP entry port.
- The `messages` check has no catalog `check` value, so it is outside the CON-23 failures rule (CON-25).

We applied the shared page notes:
- The ntp, mqtt, ldap and hls modules get their workflows and mk files.
- [Modules](../Modules.md) drops the sip_rtp agent limit.
- [Contracts](../Contracts.md) says the report format is 0.12.0.
- [Testing](../Testing.md) names the new tests.
- [Home](../Home.md) has the batch 28 status line.

The catalog gains:
- AGT-21: NTP, MQTT, LDAP and HLS in the agent.
- CON-25: the messages check in the metric catalog. It needs owner approval.
- CTL-42: the CON-23 mapping table in the controller.
- QA-10: Gate 1 without its own reuse rule.

RLM-15 also covers the HLS segment cadence inside a keepalive connection.

### Shared page notes

None.
