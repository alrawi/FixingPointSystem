## [2026-11-09] plan | Batch 16 merged

The coordinator merged CTL-31 in PR #279, CTL-30 in PR #280, CTL-14 in PR #282 (after resolving a fakeagent conflict with CTL-30), AGT-07 in PR #283, and ENG-17 in PR #284, and fixed a start race in the runs reason test in PR #285 that failed ENG-17's CI. IMG-06 merged in PR #281 after the owner host boot test passed for every role (all in one 43 checks, controller 37, agent 40, and 42 against a real host controller with the agent online over mutual TLS); `mk/img.mk` now lints its two new scripts.

We applied the shared page notes to [Testing](../Testing.md), [Home](../Home.md), and `modules/dns/README.md`. The catalog gains AGT-11 (WebAssembly extensions in the agent), CTL-32 (capture file retention), and CON-15 (capture and criteria contract gaps, which needs owner approval).

### Shared page notes

None.
