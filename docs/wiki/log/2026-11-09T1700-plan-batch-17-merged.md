## [2026-11-09] plan | Batch 17 merged

The coordinator merged CTL-32 in PR #291, CTL-12 in PR #292, OPS-05 in PR #293, and AGT-09 in PR #294, and stabilized the refused certificate test of the agent in PR #295, which had failed three times on unrelated PRs because of the TLS 1.3 alert and close race. IMG-03 (PR #290) waits for the owner host test, and its scripts join `mk/img.mk` on its branch.

We applied the shared page notes to [Controller](../Controller.md), [Contracts](../Contracts.md), [Testing](../Testing.md), and [Home](../Home.md). The catalog gains AGT-12 (a port filter for captures and a real end to end capture, which the REST contract needs before a capture can start against the real agent), CTL-33 (a version stamp in fps-controller for releases), and CTL-34 (byte weights in the compiler). AGT-10 now also owns the agent side fix of the TLS 1.3 race.

### Shared page notes

None.
