# Handoff MOD-19 DNS zone file setting

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #142 |
| Branch | `wp/MOD-19` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of N plan steps) |
| Current holder | claude-571e77, lease until 2026-11-06T17:29:13Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A test configuration names a zone file through `modules.dns.settings.zone_file` (decision D12 item 22), and the dns server answers from it. Deliver the `zone_file` setting in the manifest settings schema, its use in the Rust server side, and a fixture that answers from a configured zone.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim commit
- [ ] 2. Manifest settings schema and the Settings type in the crate, with tests
- [ ] 3. fixture_peer and capture.sh pass the setting, README and wiki page
- [ ] 4. Checks, PR, CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Write exactly one concrete step that the next agent can start immediately.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

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
| 2026-11-06T16:00Z | claude-571e77 | claim | | Claimed, branch from origin/main 3243a04 |
