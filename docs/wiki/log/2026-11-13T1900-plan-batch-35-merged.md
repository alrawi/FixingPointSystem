## [2026-11-13] plan | Batch 35 merged, Gate 2 signed off

The owner signed off Gate 2 on 2026-11-13 and approved batch 35 the same day, with RLM-23 added to it. [Gates](../Gates.md) and [Home](../Home.md) record the sign-off.

The coordinator merged the following.
- OPS-07 in PR #552: `fps_swarm.py takeover` accepts a blocked item, and the sweep names blocked items that wait for a person. [Swarm Protocol](../Swarm-Protocol.md) gained both sentences of its notes.
- QA-13 in PR #556: Zeek raises `event_queue_overflow_drain` when its event queue overflows at the end of the mix capture, and the suite now runs Zeek without the queue limit. The "Watch out for" entry of [Module Development](../Module-Development.md) now gives the cause and the setting.
- RLM-23 in PR #554: `expect_every_session` on a session rule, report format 0.21.0. Before merge the coordinator applied its text to Module Development, and reran a `ui` job that failed on an unrelated timing test.
- MOD-30 in PR #553: the smtp and imap capture scripts name their STARTTLS port. The coordinator merged main into the branch so that its Gate 2 mix ran with the QA-13 fix.
- MOD-31 in PR #555: the hls segment cadence and bytes are labels, and `hls_check.py` is gone. Before merge the coordinator changed `realism/tests/test_http_rungs.py` to keep the recorded bit rates, applied the [Realism](../Realism.md) notes, and merged main for RLM-23.

What the batch found:
- The strike fixture of MOD-28 failed its session structure check twice: the last `conn_churn` instance had 25 of its 32 churn connections, and its keeper connection ended at 0.134 s. The capture appears to stop before the spawned flows of the last instance finish. The strike module belongs to the owner.
- The http2 decrypted view counted twice the expected messages in one session, in the `harness` job and in the qa-gate2 `modules` job.
- A UI test of the run view and a kernel backend port range test each failed once.
- QA-13 offers a named rule in the parser error check for the queue weird without a session. It is not needed while the wrapper is in place.

### Shared page notes

None.
