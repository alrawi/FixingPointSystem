# Handoff MOD-03 Content generators

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #170 |
| Branch | `wp/MOD-03` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-2e55b9, lease until 2026-11-06T22:35:35Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship synthetic content generators for HTML, JSON APIs, multilingual text, email bodies, and binary file types as native template functions of the module `content`, with size parameters and seeded variety, and a provenance note that states every generator is synthetic. Acceptance: the duplicate payload ratio across 10,000 generated bodies is at most 1%.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Crate skeleton, catalog, argument binding, manifest, word lists
- [ ] 2. HTML and JSON generators
- [ ] 3. Text and email generators
- [ ] 4. Binary file generators
- [ ] 5. Tests: determinism, size, duplicate ratio over 10,000 bodies per generator, catalog equals manifest
- [ ] 6. README provenance note, mk file, Modules page, log entry, PR

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Write exactly one concrete step that the next agent can start immediately.

## Blockers and open questions

none

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T22:05Z | claude-2e55b9 | claim | | Claimed #170 with 6 plan steps |
