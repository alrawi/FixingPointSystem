## [2026-11-17] ingest | bug-592 The run feed fills every gap and the reconnect test waits for it

We fixed bug #592, the intermittent failure of the UI test "reconnects with Last-Event-ID after a dropped stream and fills the gap". The UI never reconnected twice or duplicated an event. Three things left a tick out of the table under load. The mock stream sent only the newest tick at each timer fire, so a late timer skipped ticks, and it now sends every tick since the last one. The run view read the gap only after a reconnect, although a tick can pass between the history read and the first event of the first connection, and the first event of every connection now reads the gap. The test asserted the table before the gap read had answered, and it now waits for the ticks of that read and then asserts, unchanged, that no tick is missing. Two new tests with an injected clock fail on the old code. Under three busy loops the test failed 30 of 30 runs before and passed 100 of 100 after, and 100 of 100 plain. The evidence is `docs/handoff/bug-592.md` on the branch `bug/592`. The fix changes no contract and adds no dependency.

### Shared page notes

Module-Development "Watch out for": none, the change is not visible to module authors.

Testing: add "A test that draws a stream and a history read must wait for the read to answer before it asserts that no tick is missing, and a mock stream must send every tick even when its timer fires late (bug-592)."
