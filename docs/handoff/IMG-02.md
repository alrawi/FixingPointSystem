# Handoff IMG-02 First boot roles and tuning

| Field | Value |
|---|---|
| Issue | #198 |
| Branch | `wp/IMG-02` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-55441d, lease until 2026-11-09T04:08:42Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A role selector driven by cloud-init data (controller, agent, all-in-one; default all-in-one) that enables only that role's services at first boot, and per-role tuning profiles (agent: hugepages, CPU isolation, sysctls, loopback notrack). Kernel command-line changes need one reboot and must be idempotent and recorded. Acceptance: boot tests confirm each role starts only its services.

## Plan

- [ ] 1. fps-role and fps-tune scripts, units, agent start changes
- [ ] 2. Offline tests (plan table, role parsing with a fake systemctl), static checks, mk/img.mk
- [ ] 3. boot-test.sh --role, vm-checks.sh per role
- [ ] 4. README, wiki Image page, log entry, PR, CI, handoff "waiting for host test"

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write the scripts.

## Blockers and open questions

none

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08T14:00Z | claude-55441d | claim | see branch | |
