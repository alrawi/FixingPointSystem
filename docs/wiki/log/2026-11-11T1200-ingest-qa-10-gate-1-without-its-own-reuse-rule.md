## [2026-11-11] ingest | QA-10 Gate 1 without its own reuse rule

`REUSE_WEIRDS`, `reused_findings`, and the `findings` argument of `evaluate` are gone from `tests/gates/gate1/gate1_report.py`. RLM-17 added `premature_connection_reuse` to `ZEEK_STATE_WEIRDS` in `realism/parsers.py`, so the parser error check of the realism report already leaves that weird out. The suite takes the `passed` flag of that check as it is and fails on every other finding. `parser_findings.py`, its tests, and the files `parser-findings.json`, `.csv`, and `.pcapng` are unchanged and still list `reused_tuple` for each finding. The unit tests drop the reuse cases and assert that a failed parser error check fails the gate. The README of the suite and [Gates](Gates.md) describe the new rule.

Findings: the old fallback `passed = report passed or (parsers ran and counted == 0)` existed only for the reuse rule and went with it. Zeek is not installed in the sandbox, so the three Gate 1 runs in a row and the corruption proof run in CI only.

### Shared page notes

- [Testing](Testing.md), line "Findings of batch 27": it says the suite does not count the weird in a session with more than one connection. Since QA-10 the realism runner leaves it out (RLM-17) and the suite has no rule of its own.
- [Home](Home.md), [Implementation Plan](Implementation-Plan.md): none. [Gates](Gates.md) is updated.
