## [2026-11-03] ingest | RLM-02 Parser error checks with Zeek and tshark

We added the `parser_errors` check to the realism runner in `realism/`. It runs tshark and Zeek out of process, joins their findings to the session table, and reports parser errors and malformed packets per application, with the report format raised to 0.2.0. We created the component page [Realism](Realism.md) and linked it from Home and the sidebar, as the coordinator approved. The evidence is `docs/handoff/RLM-02.md` and the tests in `realism/tests/`, which include a run of the real tshark 4.2.2 on a capture with one malformed DNS message that reports exactly one parser error.

### Shared page notes

- [Testing](Testing.md) row `test-rlm` should read "the realism runner tests, including the ndpiReader, tshark, and Zeek tests when the tools are installed".
- The Realism report section of [Implementation Plan](Implementation-Plan.md) can name the `parser_errors` check next to the conformance check.
- The metric catalog lacks a parser error metric. Issue #89 proposes `realism.parser_errors` with the labels `application` and `parser` for a CON package.
