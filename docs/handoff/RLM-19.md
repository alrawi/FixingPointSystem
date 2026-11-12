# Handoff RLM-19 The port collision rule in the realism runner

| Field | Value |
|---|---|
| Issue | #510 |
| Branch | `wp/RLM-19` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-r19a7c, lease until 2026-11-12T19:03:12Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The parser error check of the realism runner applies the port collision rule of QA-11, reports each skipped finding, and the Gate 2 suite uses the runner's rule instead of its own. A capture whose client port collides with a dissector passes with the finding reported as not counted, and a real malformed packet still fails. The Gate 2 change is outside `realism/`, so it goes to the coordinator in the Shared page notes of the log entry.

## Plan

- [x] 1. Move the rule into `realism/parsers.py` (`port_bindings`, `parse_decodes`) and `realism/parser_errors.py` (`port_collision`, `not_counted`), report format 0.18.0 and schema.
- [x] 2. Tests (`realism/tests/test_port_collision.py`, real tshark included), module job summary line, README and Realism page.
- [x] 3. Log entry with the Shared page notes (Gate 2 patch, Module Development text, controller schema copy).
- [ ] 4. Pull request, CI, review.

## Done and verified

- `python3 -m pytest realism/tests` gives 829 passed and 9 skipped, `ruff format --check`, `ruff check`, and `mypy` on `realism` pass, and `plan.py check`, `render --check`, and `lint` pass.
- The Gate 2 change was tried in place and kept as a patch (62 tests of `tests/gates/gate2` pass with it), then reverted, because the paths are not ours.

## Remaining

The controller copy of the schema and `ui/src/api/schema.d.ts` need a refresh that is outside the paths (see the log entry), so the controller and ui jobs are expected to be red until the coordinator applies it.

## Next action

Open the pull request, poll the check runs, and report the failing jobs that are outside the paths.

## Blockers and open questions

none

## Decisions taken in this package

- Report format 0.18.0, three optional fields on the `parser_errors` entry, because a 0.17.0 reader that closes the entry would refuse them.
- The table is read when the parser check starts and not only when a finding exists, so `port_collision_ports` says that the rule was active on a clean capture too.
- A failed `tshark -G decodes` gives an empty table, so every finding counts (the rule never makes the check looser by failing).
- The rule does not compare the layer of the finding with the dissector, because the names of `tshark -G decodes` and the protocol of the faulty layer differ (`pmproxy` and `pcp`).

## Pitfalls

- The worktree guard refuses heredocs in a command that also names git, so edits go through scripts without git or through the editor tools.
- `ruff format` on `tests/gates/gate2` reformats more than the lines of a patch, so the Gate 2 patch carries some format changes.

## Verify the current state

```bash
python3 -m pytest -q realism/tests/test_port_collision.py realism/tests/test_parser_errors.py   # 16 + the old tests pass
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T18:03Z | claude-r19a7c | claim | none | lease taken on #510 |
