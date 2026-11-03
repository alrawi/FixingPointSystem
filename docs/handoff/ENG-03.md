# Handoff ENG-03 Template compiler with module functions

| Field | Value |
|---|---|
| Issue | #23 |
| Branch | `wp/ENG-03` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-eng-03-4b2d, lease until 2026-11-03T04:14:01Z |
| Last verified commit | none |

## Goal

Compile templates of contract v0.2 into byte segments and call slots, dispatch module function calls through a `TemplateFn` trait, fold templates without session dependent parts into constants, and print a human readable program dump for golden tests.

## Plan

- [ ] 1. Parser for the v0.2 grammar with nested calls, keyword arguments, float, boolean, and `last`
- [ ] 2. Segment compiler with core functions, module call slots, and constant folding
- [ ] 3. The `TemplateFn` trait and dispatch at render time
- [ ] 4. Program dump and golden tests for every grammar rule
- [ ] 5. Property test, Engine page, log entry

## Done and verified

None yet.

## Remaining

All steps.

## Next action

Write the v0.2 expression parser in `engine/fps-flow/src/template.rs`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cargo test -p fps-flow
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-eng-03-4b2d | claim | none | card 6594102239 |
