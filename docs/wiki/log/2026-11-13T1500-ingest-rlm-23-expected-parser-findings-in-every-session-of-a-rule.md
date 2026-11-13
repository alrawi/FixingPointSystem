## [2026-11-13] ingest | RLM-23 Expected parser findings in every session of a rule

A session rule of the labels can now hold each of its sessions to its `expect_findings` (issue #551, the remainder of #513). The rule takes the optional boolean `expect_every_session`. Without it, or when false, RLM-20 applies unchanged: one finding in any session of the rule meets a declared item. With `expect_every_session: true`, every session that the rule matches first must show every declared item of a parser that ran. The parser error check (`realism/parser_errors.py`) adds one entry to `missing_findings` for each session that lacks an item. The entry has the RLM-20 fields (`session_rule`, `application`, `parser`, `item`) and `session`, `transport`, `client`, and `server`, and each entry adds 1 to `realism.check_failures`. The report format is 0.21.0, and the four fields are optional in the schema, so reports of 0.20.0 and older still validate. The labels schema has the new property, and the labels file stays at `version: 1`.

Design choices:

- A rule level boolean, not a count (`max_count` of the #513 sketch) and not a per item flag. The question that a strike module asks is "is each session of this scenario malformed", and a boolean answers it without a number that drifts when the scenario sends more requests.
- The setting needs `expect_findings` in the same rule and is refused otherwise, so a typo cannot silently do nothing.
- Only the entries of a rule with the setting carry the session fields, so a rule without it reports exactly what RLM-20 reported. A rule with the setting that matches no session gives one entry per item without a session, as RLM-20 does, so an empty rule still fails.
- Expected findings, undeclared findings, the port collision rule, and unchecked items of a parser that did not run work as in RLM-20. A finding of one session never meets the item of another.
- Matching on payload content stays out, as the plan decided: the session table holds no payload, and a separate server port per scenario selects the sessions.

Evidence: `realism/tests/test_expected_findings.py` has 14 new test cases. A capture with three malformed DNS sessions and findings for two of them fails under the setting and names the third by session id and endpoints, and the same capture passes without the setting and with `expect_every_session: false`. Further tests cover a passing capture, several lacking sessions, items and parsers, a finding that stays in its own session, another rule without the setting next to one with it, a rule that matches no session, an item of a parser that did not run, the report schema with and without the session fields (and a session entry without its client failing), the refused values, and the labels schema agreeing with the parser. One test runs the real tshark on two malformed DNS answers and one clean lookup: with the setting the clean session is named, without it the capture passes. The 0.20.0 version assertions of the other realism tests follow the bump. `make lint-rlm test-rlm` passes (ruff, mypy, and 935 passed with 9 skipped). Every committed labels file loads, and the dns, http1, ftp, and hls fixtures still score with exit 0 under tshark.

Findings:

- The controller schema copy (`controller/internal/report/schema/realism-report.schema.json`, `go generate` target, equality tested by `realism_test.go`) and `ui/src/api/schema.d.ts` (regenerated with openapi-typescript 7.13.0, the pinned version, and only the expected lines changed) are refreshed in this pull request, as RLM-20 and RLM-22 did. They lie outside `realism/`, and the Go test and `npm run check:api` need them. No Go code could be built here, and the copy was made with `cp`.
- No committed labels file declares `expect_findings` yet, so no fixture changed. The strike module (MOD-28) can set `expect_every_session: true` on each malformed scenario's rule once that scenario has its own `server_port`.
- Zeek was not installed here. The Zeek side of the per session check is covered with stub parsers only.

Shared page notes:

- [Module Development](Module-Development.md), section "Fixtures and realism" (the labels of a fixture are the interface of a module author; MOD workstream, not in the paths of this package). In the paragraph that starts "A module whose traffic is malformed on purpose", replace the sentence "One finding of any session of the rule meets an item." with this one:

  > Unless the rule sets `expect_every_session`, one finding of any session of the rule meets an item.

  Then add this paragraph directly after that paragraph and before the paragraph that starts "What the repository does for first party modules goes further":

  > When every session of a scenario must be malformed, set `expect_every_session: true` on its rule. Every session that the rule matches first must then show every declared item of a parser that ran, and the entry of `missing_findings` for a session that lacks one names the session (`session`, `transport`, `client`, `server`), so a scenario that comes out malformed in only some of its sessions fails and you can find the session in the capture. The setting needs `expect_findings` in the same rule. A finding of one session never meets the item of another, so declare only items that every session of the rule produces. The rule selects sessions by transport, addresses, and ports only, so give each malformed scenario its own server port and its own rule, as above, and do not set `expect_every_session` on a rule that also matches a well formed scenario, because its sessions would lack the items.
  >
  > ```yaml
  > sessions:
  >   - application: http1
  >     match: {transport: tcp, server_port: 8080}
  >     expect_findings:
  >       tshark: ["http: Illegal characters"]
  >     expect_every_session: true
  > ```

- [Module Development](Module-Development.md), section "Watch out for". Replace the entry "A declared item is met by one session." with this entry (the entry that stays above it, "Malformed scenarios that share a server port share one rule", is unchanged):

  > **A declared item is met by one session.** An item of `expect_findings` counts as present when any one session of its rule shows it, so a scenario that comes out malformed in only some of its sessions still passes. Set `expect_every_session: true` on the rule of a scenario whose sessions must all be malformed: every session of the rule must then show every declared item, and `missing_findings` names each session that lacks one. The setting cannot tell scenarios apart that share a server port, so give each malformed scenario its own port and rule first (RLM-20, RLM-23).

- [Testing](Testing.md): the realism entry for `realism/tests/test_expected_findings.py` can add the per session setting (`expect_every_session`, one test with the real tshark that skips without it).
- [Home](Home.md), [Implementation Plan](Implementation-Plan.md), [Swarm Protocol](Swarm-Protocol.md), [Coding Standards](Coding-Standards.md): none. [Realism](Realism.md) is updated.
