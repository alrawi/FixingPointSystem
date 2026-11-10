# Handoff CON-19 Contract gaps found by CAP-02 and RLM-06

| Field | Value |
|---|---|
| Issue | #362 |
| Branch | `wp/CON-19` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (4 of 5 plan steps) |
| Current holder | claude-51af6f, lease until 2026-11-10T06:15:04Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Close four contract gaps, all additive. CAP-02 found no integer to bytes function, no clock function, and no rule for variables across `after`. RLM-06 found that the test configuration has no arrival or size distributions, and that the metric catalog has no `all` value for `application` and no `both` value for `property`. The owner approved the contract change.

## Plan

- [x] 1. Template functions `int_bytes` and `now` and the `after` variable rule: README, validator, tests, scenario schema description (scenario contract v0.6)
- [x] 2. Arrival and size distributions in the test configuration schema (v0.3), validator rules, tests
- [x] 3. Metric catalog 0.3.0: `all` for `application`, `both` for `property`
- [x] 4. `contracts/changes/CON-19.md`, CHANGELOG, README, embedded copies, generated code, wiki Contracts page
- [ ] 5. Run the checks, open the PR, wait for CI

## Done and verified

- `python3 -m pytest tools/contracts -q` and `python3 tools/contracts/validate.py`: 290 passed, 1 skipped, 0 validator errors.
- `python3 tools/plan/plan.py check`, `render --check`, and `lint`: pass.
- `cargo test -p fps-stats --test catalog`: 4 passed (the `LABEL_VALUES` mirror gained `both`).
- `python3 -m pytest realism/tests/test_temporal.py`: 58 passed.
- `python3 sdk/python/scripts/generate_client.py --check`: unchanged. `ui/src/api/schema.d.ts` regenerated with openapi-typescript 7.13.0 (only the new definitions differ).
- The three controller copies (scenario schema, test configuration schema, catalog) are byte identical to the contract files, and a Python test compares them.

## Remaining

Open the PR and wait for CI.

## Next action

Open the pull request against `main` and poll the checks.

## Blockers and open questions

none

## Decisions taken in this package

- `after` carries variables, per role. The engine already merges the variables of a completed side and starts later flows with a copy, and the two roles run on different agents with no channel, so the rule promises visibility only on the same role.
- `now` is evaluated per call, with a pinned clock for goldens and replays. Its output length is fixed, so byte counts replay without pinning. `int_bytes` is unsigned only, width 1 to 8, with an optional endianness that defaults to big.
- The arrival and size fields are declarations that realism follows. They add no agent command.
- `all` is documented on the open `application` label, and the module name `all` is reserved in the validator.
- The `LABEL_VALUES` mirror in `engine/fps-stats/src/catalog.rs` gained `both`, because its contract test compares it with the catalog file. This is the only edit outside the catalog paths.

## Pitfalls

- The controller compiler decodes the test configuration with `KnownFields(true)`, so a configuration that declares `arrival` or `size` fails there until a CTL package accepts the fields.
- Shared engine fixtures (`verdicts.yaml`) must not hold `int_bytes` or `now` until ENG implements them, because the same fixtures run in the Rust test.

## Verify the current state

```bash
python3 -m pytest tools/contracts -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T04:48Z | claude-51af6f | claim | none | Claimed #362, branch from origin/main 7975e20 |
