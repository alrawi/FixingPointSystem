# Handoff ENG-07 Multi flow scenario instances

| Field | Value |
|---|---|
| Issue | #27 |
| Branch | `wp/ENG-07` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-eng-07-74e3, lease until 2026-11-03T16:39:34Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add a scenario instance runner to `fps-interp` that owns shared scenario variables and flow states, runs flows in `after` order and through `spawn`, and fails the instance when a spawned flow fails.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim, branch, handoff, and pull request
- [ ] 2. Read contract v0.2 rules for after, spawn, set, and failure propagation
- [ ] 3. `ScenarioInstance` with flow states, ordering, spawn, and shared variables
- [ ] 4. Tests for web_browse ordering and spawned flow failure
- [ ] 5. Wiki ingest, log entry, final checks, and review

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T16:10:00Z | claude-eng-07-74e3 | claim | | Card 6607414675 |
