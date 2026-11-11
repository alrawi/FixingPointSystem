## [2026-11-11] ingest | AGT-15 stabilized the agent enrollment test

AGT-15 fixed the flaky `an_agent_enrolls_with_a_token_registers_and_renews` in `engine/fps-agent/tests/mtls.rs`. The change touches only the test file.

Cause, two races in the fake enrollment endpoint and the test:
- The endpoint answered the enrollment with `renew_at` 1500 ms ahead, and the agent renewed on its own clock. The test waited for a session and then asserted that the endpoint had seen exactly one CSR name. A runner that needed more than 1.5 s to open the session let the renewal push a second name first, so the assertion at line 550 failed. The renewal could also replace the certificate before the test read it as `first`.
- The endpoint reset its delay to one hour after it read it, so the answer to the first renewal still carried the 1500 ms delay and the agent renewed a second time. The wait for `renews == 1` therefore depended on seeing the counter inside a 1.5 s window.

Fix: the endpoint holds `/renew` requests until the test releases them, so the test reads the enrollment state before the renewal, and the answer to a renewal always carries the one hour delay, so the agent renews once. No sleeps were added. With the delay cut to 1 ms the old test fails at line 550 on every run; the fixed test passed 200 runs in a row and 100 more under four busy loops.

Shared page notes: none. Agent.md, the Testing row of the component tests, needs no change beyond the sentence that this package added.
