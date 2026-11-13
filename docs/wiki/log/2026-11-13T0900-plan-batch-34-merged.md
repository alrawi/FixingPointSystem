## [2026-11-13] plan | Batch 34 merged

The owner approved batch 34 and CON-31 on 2026-11-12, and decided that flows ordered with `after` also belong to the scenario session.

The coordinator merged the following. On each branch it first applied the package's text to [Module Development](../Module-Development.md), as the module documentation rule asks.
- RLM-21 in PR #539: STARTTLS ports in the decrypted view.
- CON-31 in PR #540: a session is one run of the entry flow, and every spawned or `after` flow belongs to it. The guide said that the engine counts this way only from ENG-30. GitHub missed the coordinator's first push to the branch, and an empty commit made it run CI.
- ENG-30 in PR #542: the engine records one session per instance with the bytes of all its flows. The guide's interim sentence went with it.
- RLM-22 in PR #541: the request cadence and the bytes per rung of HLS in the labels, report format 0.20.0.
- CTL-49 in PR #543. The controller needed no change once ENG-30 had merged. qa-gate2 passes on the bytes basis with all 15 applications and no waiver: the largest mix difference is 1.347 points by the run metrics and 1.274 points by nDPI, against 2 points. The coordinator relabelled the issue from blocked to ready, because `fps_swarm.py takeover` refuses a blocked item (OPS-07), and corrected the waiver text of Gates and the suite README before merge.

Gate 2 now holds in full in CI and waits for the owner's sign-off.

What the workers found:
- `smtp` and `imap` capture scripts still rely on the TLS heuristic (MOD-30).
- `hls_check.py` can now move into labels (MOD-31).
- The catalog has no series for the bytes per rung. A CON package could add one if the run report should carry it.
- A Zeek resource weird at the end of a large capture failed one Gate 2 run (QA-13).

We applied the shared page notes:
- [Testing](../Testing.md) names the batch 34 tests and the flake.
- [Home](../Home.md) has the batch 34 status line.

The catalog gains MOD-30, MOD-31, OPS-07 and QA-13.

### Shared page notes

None.
