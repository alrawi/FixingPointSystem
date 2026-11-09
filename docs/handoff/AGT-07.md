# Handoff AGT-07 Module settings, side label, and client default in the agent

| Field | Value |
|---|---|
| Issue | #216 |
| Branch | `wp/AGT-07` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-a7c3e1, lease until 2026-11-09T14:07:01Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply `ModuleRef.settings` in the agent (dns `zone_file`, relative paths resolved against and confined to the module directory), register the UDP session series through `fps_stats::SessionCounters` with the side label, and make the agent criteria count the client side unless a side is named.

## Plan

- [ ] 1. Module settings reach the dns native functions, with path confinement and tests
- [ ] 2. UDP worker registers through SessionCounters; snapshots hold both sides; criteria default to the client side
- [ ] 3. Controller e2e run of a dns zone test in controller_e2e.sh
- [ ] 4. Agent wiki page, log entry, checks, PR, CI

## Done and verified

Nothing yet.

## Remaining

All steps.

## Next action

Start step 1 in `engine/fps-agent/src/modules.rs`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-a7c3e1 | claim | none | Claimed #216 |
