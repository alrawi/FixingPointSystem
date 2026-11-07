## [2026-11-07] plan | Batch 12 merged

Sonnet worker agents built batch 12, and the coordinator merged ENG-15 in PR #233, ENG-16 in PR #234, ENG-13 in PR #235, ENG-12 in PR #236, CTL-23 in PR #238, and CTL-22 in PR #237. A stopped launch left leases on CTL-22 and CTL-23, which the coordinator released before it dispatched them again. CTL-23 found that the two intermittent controller tests raced in their own assumptions and fixed both, and CTL-22 merged after it picked up that fix. IMG-01 now builds a static musl agent and a static controller, so the image no longer depends on the host's C library, and it waits for the owner host test.

We added the `eng-capture` workflow, which runs the tshark tests of `fps-capture` with `FPS_REQUIRE_CAPTURE_TOOLS=1`, a row for it in [Testing](../Testing.md), and the Home status. The catalog gains ENG-18 (rate handles and paused worker wake in the runtime, which AGT-05 now depends on), CTL-25 (scenario contract v0.3 in the controller registry), and CON-12 (side selection in the run metrics API).

### Shared page notes

None.
