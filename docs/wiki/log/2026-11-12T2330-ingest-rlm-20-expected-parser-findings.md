## [2026-11-12] ingest | RLM-20 Expected parser findings in the realism labels

The labels of the realism runner can now say that a parser finding is expected (issue #491). A session rule takes `expect_findings`, a mapping from `tshark` or `zeek` to a list of items. A Zeek item is a weird name (`bad_HTTP_request`). A tshark item is the protocol of the frame with the expert item (`http`), optionally followed by a colon, a space, and a part of the expert message (`http: Illegal characters`). The parser error check (`realism/parser_errors.py`) treats a finding that a rule of its session declares as expected: it stays out of `errors_total`, `parser_errors`, `malformed_packets`, the metric series, and the verdict, and the entry lists it under `expected_findings` (first 100, with `session_rule`) and `expected_total`, next to `not_counted` of RLM-19. Two things fail the check and the report lists both: a declared item that no session of its rule produced (`missing_findings`, one `realism.check_failures` each), and a finding that no rule declares (`errors`, as before). A declared item of a parser that did not run goes to `unchecked_expectations` and does not fail. Expected findings come before the port collision rule. Report format 0.19.0.

Design choices: declarations per rule and per item, and no tolerated count, so that a malformed scenario cannot hide a second fault and a scenario that stops being malformed fails. The first matching rule applies, as for the application. A Zeek finding merges the weirds of one connection and packet time, and it is expected only when every weird is declared. An item is met by one finding in any session of its rule. tshark keeps one finding per frame with the message of its first error item, so the message part is matched against that one.

Evidence: `realism/tests/test_expected_findings.py` (23 tests): a declared finding passes and is reported, a missing one fails, an undeclared one next to a declared one fails, another rule's declaration does not cover a session, the message part and the protocol of a tshark item, partial Zeek coverage, per parser checks, unchecked items, precedence over the port collision rule, the cap of 100, the schema closing the new entries, refused declarations, the labels schema agreeing with the parser, and one test with the real tshark on a malformed DNS answer (declared passes, undeclared fails, wrong item fails, clean capture with a declaration fails). The 0.19.0 version assertions of the other tests follow the bump.

Findings:

- The strike module (issue #491, MOD-28) can now declare `bad_request_line` and `te_cl_conflict` per rule. The Zeek names and the tshark protocol and message of each must be taken from a real capture of the harness, because this package ran no Zeek (not installed here) and only a synthetic malformed DNS answer with tshark 4.2.2.
- An item is met by one finding in any session of its rule. A rule over many sessions therefore does not prove that every one of them is malformed. A per session count would be a later extension.
- The controller schema copy and `ui/src/api/schema.d.ts` are refreshed in this pull request (coordinator authorized). Reports of 0.18.0 and older still validate, because the four fields are optional.

Shared page notes:

- [Module Development](Module-Development.md), section "Fixtures and realism" (the labels of a fixture are the interface of a module author; MOD workstream, not in the paths of this package). Add this paragraph directly after the paragraph that starts "A fixture score can list a parser finding as not counted" and before the paragraph that starts "What the repository does for first party modules goes further":

  > A module whose traffic is malformed on purpose, such as a scenario that sends an invalid request line, produces parser findings that you want, and the check must not count them as faults. Declare them in the label rule of the session. A rule takes `expect_findings`, which lists per parser the items that its sessions must produce.
  >
  > ```yaml
  > sessions:
  >   - application: http1
  >     match: {transport: tcp, server_port: 8080}
  >     expect_findings:
  >       zeek: [bad_HTTP_request]
  >       tshark: ["http: Illegal characters"]
  > ```
  >
  > A Zeek item is the name of a weird, as `weird.log` prints it in its `name` column. A tshark item is the protocol of the frame with the expert item, which is the last name in the `frame.protocols` field of that frame (`http`, `dns`, `tcp`), and optionally a colon, a space, and a part of the expert message, which the first expert item of error severity of the frame must contain. List exactly what a recording of your scenario shows: take the items from the `errors` of the `parser_errors` entry of a first score that has no declaration, and check each against the traffic that you meant to send. A declared finding does not count, and the entry lists it under `expected_findings`. A declared item that no session of the rule produced fails the check and is listed under `missing_findings`, so a scenario that stops being malformed fails, and a finding that no rule declares fails as before, so a second fault in the same scenario does not hide behind the first. The first rule that matches a session applies, so declare the items on the rule that the sessions of the malformed scenario match first, and keep well formed scenarios on their own rules. One finding of any session of the rule meets an item. When a parser is not installed, its items are listed under `unchecked_expectations` and do not fail the check, so score with `--parsers tshark,zeek` to require both (`realism/README.md`, Parser errors).

- [Testing](Testing.md): the realism runner list can add `realism/tests/test_expected_findings.py` (the expected findings of the labels, one test with the real tshark that skips without it).
- [Home](Home.md), [Implementation Plan](Implementation-Plan.md), [Swarm Protocol](Swarm-Protocol.md), [Coding Standards](Coding-Standards.md): none. [Realism](Realism.md) is updated.
