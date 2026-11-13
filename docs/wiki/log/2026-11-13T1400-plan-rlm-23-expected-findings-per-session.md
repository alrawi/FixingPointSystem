## [2026-11-13] plan | RLM-23, expected findings per session

Issue #513 asked that a strike scenario declare the parser findings its malformed traffic should produce. RLM-20 (#531) merged four hours after the issue was filed and covers most of it: `expect_findings` on a session rule, missing items failing, undeclared findings failing, and an `expected_findings` section in the report.

The owner agreed on 2026-11-13 to the remaining plan:
- RLM-23 adds an optional rule setting under which every session of the rule must show every declared item. RLM-20 is met by any one session.
- Matching on payload content, such as the request method, stays out. The session table holds no payload, and a separate server port per malformed scenario selects the sessions.
- A finding without a session cannot be declared. It is scoped only if a strike fixture shows one.

[Module Development](../Module-Development.md) gains two entries in Watch out for: malformed scenarios that share a server port share one rule, and a declared item is met by one session.

### Shared page notes

None.
