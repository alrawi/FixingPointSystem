# Handoff MOD-25 Byte estimates in every module manifest

| Field | Value |
|---|---|
| Issue | #494 |
| Branch | `wp/MOD-25` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-9c4e1a, lease until 2026-11-12T17:54:55Z |
| Last verified commit | none |

## Goal

Every scenario of every first party module manifest carries a measured `expected_bytes` entry, each with a guard test that fails at a 2 percent difference. `FPS_GATE2_BASIS` leaves `qa-gate2.yml` so that Gate 2 proves the bytes basis with no waiver. `modules/strike/` and `modules/_template/` are out of scope (the template estimate is checked only).

## Plan

- [ ] 1. Pattern and harness survey; claim commit
- [ ] 2. dns, ntp, ldap, mqtt estimates and guards
- [ ] 3. ftp, smtp, pop3, imap estimates and guards
- [ ] 4. http1, http2, ssh, smb2 estimates and guards
- [ ] 5. Guide fixes, qa-gate2.yml, log entry
- [ ] 6. PR, CI, review

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Survey the test harness of each module.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
git log --oneline -3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T16:25Z | claude-9c4e1a | claim | none | claimed #494 |
