# Handoff CON-32 Kernel release, selected backend, and XDP mode in the agent capabilities

| Field | Value |
|---|---|
| Issue | #576 |
| Branch | `wp/CON-32` |
| Pull request | none yet |
| Status | in progress |
| Progress | 5% (0 of 4 plan steps) |
| Current holder | claude-8eee15, lease until 2026-11-17T01:15:17Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Additive contract change approved by the vetting agent under D13 (issue #74 in part). Add `Capabilities.kernel_release` (9), `Capabilities.selected_backend` (10), and `PortInfo.xdp_driver_mode` (6) with the enum `XdpMode`, and their REST mirror. Agent protocol 0.2.4 to 0.2.5 and an OpenAPI minor bump. Write `contracts/changes/CON-32.md` with a Consumers table, AGT-22 first.

## Plan

- [ ] 1. Claim, branch, handoff file
- [ ] 2. Proto fields, enum, and version 0.2.5 in the proto header, buf.yaml, and README
- [ ] 3. OpenAPI mirror and version 0.13.0, validators and tests
- [ ] 4. Change entry, wiki ingest, log entry, checks, pull request, and CI

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Edit `contracts/proto/fps/agent/v1/agent.proto`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest tools/contracts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16 | claude-8eee15 | claim | none | Claimed #576, branch wp/CON-32 from origin/main 9a6c495 |
