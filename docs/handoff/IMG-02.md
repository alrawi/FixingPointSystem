# Handoff IMG-02 First boot roles and tuning

| Field | Value |
|---|---|
| Issue | #198 |
| Branch | `wp/IMG-02` |
| Pull request | #258 |
| Status | waiting for host test |
| Progress | 95% (CI green, host test pending) |
| Current holder | claude-55441d, lease until 2026-11-09T04:08:42Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A role selector driven by cloud-init data (controller, agent, all-in-one; default all-in-one) that enables only that role's services at first boot, and per-role tuning profiles (agent: hugepages, CPU isolation, sysctls, loopback notrack). Kernel command-line changes need one reboot and must be idempotent and recorded. Acceptance: boot tests confirm each role starts only its services.

## Plan

- [x] 1. fps-role and fps-tune scripts, units, agent start changes
- [x] 2. Offline tests (plan table, role parsing with a fake systemctl), static checks, mk/img.mk
- [x] 3. boot-test.sh --role, vm-checks.sh per role
- [ ] 4. README, wiki Image page, log entry, PR, CI, handoff "waiting for host test"

## Done and verified

`shellcheck -x` clean on every script. `make lint-img test-img`: pass, including `image/tests/role-test.sh` (13 checks) and `image/tests/tune-test.sh` (29 checks: sizing table, apply logic with fakes, idempotence, no reboot loop). `plan.py check`, `render --check`, `lint`: ok. NOT verified here (no KVM): the boot test per role, `update-grub` on the real image, that `hugepages=` and `isolcpus` take effect, the systemd ordering and conditions on a real boot.

## Remaining

CI green on the PR, then the owner host test of the three roles (commands in the PR description).

## Next action

Open the PR and wait for CI. Then: on the KVM host run `image/build.sh`, `image/boot-test.sh` (all-in-one, must keep the 26 checks), `image/boot-test.sh --role controller`, `image/boot-test.sh --role agent`, or `--role all`.

## Blockers and open questions

none

## Decisions taken in this package

Role comes from `/etc/fps/role.env` (cloud-init `write_files`), parsed by `fps-role`, never sourced. Units use `ConditionPathExists` on markers in `/run/fps/role.d/` plus `systemctl enable/disable --no-reload`. All-in-one gets sysctls and notrack only (no kernel option, no reboot) so the default stays as IMG-01. Agent: `isolcpus/nohz_full/rcu_nocbs` from CPUS/4 housekeeping CPUs (none below 4 CPUs), `hugepages=` 25 percent of memory in 2 MiB pages, agent pinned by `taskset`. One reboot, guarded by `reboot_requested_for` in `/etc/fps/tune.applied`, so a failed apply never loops. The agent role test VM needs 4 CPUs and 4 GiB.

## Pitfalls

Unverified on a real boot: whether `systemctl reboot` from `fps-tune.service` interrupts cloud-init's final stage (it only matters for user `runcmd`), and whether `update-grub` is present in the image. If the agent boot test fails at the reboot wait, read `/etc/fps/tune.applied` (`detail=`) and `journalctl -u fps-tune`.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08T14:00Z | claude-55441d | claim | see branch | |
| 2026-11-08T16:00Z | claude-55441d | implement, tests, docs | see branch | Offline tests pass, waiting for PR CI |
| 2026-11-08T16:20Z | claude-55441d | PR 258 opened, CI green | see branch | Waiting for host test, issue stays claimed |
