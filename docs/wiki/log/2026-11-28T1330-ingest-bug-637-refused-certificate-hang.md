## [2026-11-28] ingest | bug 637 A refused certificate does not always end the agent

Issue #637: `a_certificate_that_the_controller_refuses_is_a_permanent_failure` (mtls.rs) timed out once in CI.

Cause. A real agent bug. `session` bounded the dial but not the `Register` call or the opening of the session stream. In about 1 run in 400 the connection closed during the setup and `Register` never completed with an error (debug prints: stuck in `Register`; `ss`: no established connection, both sides gone). No session ended, so `RedialPolicy` never ran and the redial limit never applied: the agent waited forever.

Fix. Both calls run under `CALL_TIMEOUT` (5 s). A timeout is the transient `Lost::Unanswered`, so the redial policy bounds it, and the next dial sees the close and ends permanently as before. The test is unchanged.

Evidence: 6 failures in 2500 runs before, 0 in 3000 runs under CPU load after.

### Shared page notes

- Module Development, "Watch out for": nothing for module authors.
- Agent page (if it lists session timeouts): `Register` and the session open are bounded at 5 s; a timeout is a transient failure that counts against the redial limit.
