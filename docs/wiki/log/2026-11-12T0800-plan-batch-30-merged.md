## [2026-11-12] plan | Batch 30 merged

The coordinator merged:
- ENG-28 in PR #489, MOD-23 in PR #488, AGT-21 in PR #484, QA-05 in PR #487 and CTL-46 in PR #483.
- CON-27 in PR #485, which the owner approved on 2026-11-11. With the coordinator's leave, the worker also updated the consumers on its branch: the catalog copy, the pins, the fps-stats tables, the host_identity series, report format 0.15.0, the controller schema copy, the CON-23 test counts and the UI types. The coordinator merged main into it after CTL-46.
- RLM-15 in PR #486. CON-27 took format 0.15.0 first, so the worker merged main and moved to 0.16.0. One qa-gate2 run failed twice on 10 parser findings in its mix stage and passed on a full rerun (QA-11).

What the workers found:
- Gate 2 verdict PASS WITH WAIVERS (QA-05):
  - The controller does not read `expected_bytes`, so the byte basis is waived (CTL-47).
  - The mix waived hls, ldap, mqtt and ntp until AGT-21. This wrap-up clears that waiver in qa-gate2.yml, so the suite's own CI run checks the full 15 application mix.
  - The thinnest margin is HTTPS, at 1.37 of 2 points by nDPI.
  - The run report counts the FTP data connection and the SIP media stream as sessions of their own.
  - `Run.Calibration` is not in the OpenAPI schema (CON-29).
- The http2 session count flake was tshark finding TLS on port 8443 or 8444 only by its heuristic, which misses a ClientHello split after its first bytes (MOD-23, runner fix RLM-18).
- The first runtime flake was a test bug: the kernel may give one source host the same port towards two destination ports. The second, 16 of 32 sessions, is not explained, and the tests now name the failed flow (ENG-28).
- Packet gaps have no KS series in the catalog (CON-28). The sip_rtp and hls timing scripts can become `temporal.packets` labels (MOD-24).
- CON-27 settles that a report holds at most one check of each kind, so the AND of `passed` per kind in `MapRealism` is unreachable.

We applied the shared page notes:
- [Testing](../Testing.md) names the Gate 2 suite and the batch 30 tests and flakes.
- [Controller](../Controller.md) notes that the scenario catalog does not read `expected_bytes`.
- [Implementation Plan](../Implementation-Plan.md) names the Gate 2 waiver.
- [Home](../Home.md) has the batch 30 status line.

The catalog gains:
- CON-28: packet gap and payload size series. It needs owner approval.
- CON-29: run calibration in the REST API. It needs owner approval.
- CTL-47: byte estimates in the scenario catalog.
- CTL-48: the CON-23 wording for every realism metric.
- QA-11: the parser findings of the Gate 2 mix stage.
- RLM-18: TLS on non standard ports in the decrypted check.
- MOD-24: packet gap labels in place of the module timing scripts.
- MOD-25: byte estimates in every module manifest.

### Shared page notes

None.
