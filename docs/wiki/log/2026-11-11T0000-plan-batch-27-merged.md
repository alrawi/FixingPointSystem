## [2026-11-11] plan | Batch 27 merged

The coordinator merged:
- CTL-40 in PR #431 and ENG-26 in PR #437.
- ENG-27 in PR #432 and RLM-14 in PR #436.
- AGT-19 in PR #438, UI-08 in PR #435 and QA-09 in PR #439.
- CTL-41 in PR #433, after merging main and refreshing its embedded copy of the realism report schema to RLM-14's format 0.11.0.
- CON-23 in PR #434. The owner chose catalog metrics per check kind over a separate verdict list on 2026-11-10 and approved the change. On its branch the coordinator also:
  - refreshed the embedded catalog copy;
  - added the seven metrics and the `check` and `parser` labels to the fps-stats tables;
  - moved the version pins in `tools/contracts` and `realism/tests/test_mix.py` to 0.5.0;
  - regenerated `controller/internal/api`, the UI types and the Python SDK.

What the workers found:
- The Gate 1 parser error flake was a reused TCP 5-tuple that the runner joined into one session (QA-09).
- The capture's `dropped` counts sessions, not packets, and mixes three causes. ENG-27 split them inside fps-capture and kept the old sum on the wire (CON-24).
- The runtime keeps one data listener per port serving every instance until each one's own peer arrives, instead of changing the port draw (ENG-26).
- Attached realism reports live in memory, like runs, so a controller restart drops them (CTL-41).
- The CON-23 schema patch was written against report format 0.10.0, before RLM-14 added the messages check (RLM-17).

We applied the shared page notes:
- qa-gate1.yml runs the suite three times in a row on pull requests (`FPS_GATE1_REPEAT`), and the gate job timeout is 60 minutes.
- [Image](../Image.md) says the controller now sends the headers at once.
- [Testing](../Testing.md) names the findings.
- [Home](../Home.md) has the batch 27 status line.

The catalog gains:
- RLM-17: the CON-23 metrics in the realism report.
- CON-24: capture drop counts in the agent protocol and the REST API. It needs owner approval.

### Shared page notes

None.
