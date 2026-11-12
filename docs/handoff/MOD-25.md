# Handoff MOD-25 Byte estimates in every module manifest

| Field | Value |
|---|---|
| Issue | #494 |
| Branch | `wp/MOD-25` |
| Pull request | #518 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-9c4e1a, lease until 2026-11-12T17:54:55Z |
| Last verified commit | 890738d |

## Goal

Every scenario of every first party module manifest carries a measured `expected_bytes` entry, each with a guard test that fails at a 2 percent difference. `FPS_GATE2_BASIS` leaves `qa-gate2.yml` so that Gate 2 proves the bytes basis with no waiver. `modules/strike/` and `modules/_template/` are out of scope (the template estimate is checked only).

## Plan

- [x] 1. Pattern and harness survey; claim commit
- [x] 2. dns, ntp, ldap, mqtt estimates and guards
- [x] 3. ftp, smtp, pop3, imap estimates and guards
- [x] 4. http1, http2, ssh, smb2 estimates and guards
- [x] 5. Guide fixes, qa-gate2.yml, log entry
- [x] 6. PR, CI, review

## Done and verified

- Guard test `default_parameters_move_the_expected_bytes` in the 14 modules with scenarios (hls had it); each passes locally with `cargo test --offline -p fps-module-<m> --test scenarios default_parameters` (tmpfs target dir, the disk was full).
- `cargo fmt --check` and `cargo clippy --tests -D warnings` clean for the 13 crates changed; `validate.py` over the module trees: 0 errors, 0 warnings (the template and tls errors are the known directory name ones).
- `plan.py check`, `render --check`, `lint` ok.
- First CI run (51af583) removed `FPS_GATE2_BASIS`: the mix FAILED on the bytes basis (ftp +7.84 pp, sip_rtp +4.42 pp; the run counts the FTP data connection and the RTP media as sessions, so calibration halves their bytes per session). Workflow restored to main, deltas in the log entry. Also `tests/gates/gate2/test_mix_config.py` (QA-05, outside my paths) fails because the ssh manifest now has more than one key; fix noted for the coordinator.

## Remaining

The acceptance item "qa-gate2 passes on the bytes basis with no waiver" does not hold (controller or run report counts spawned flows as sessions), and the QA-05 unit test needs a one line fix. Both are for the coordinator, see the PR description and the log entry.

## Next action

Coordinator: push the one line fix of tests/gates/gate2/test_mix_config.py line 132 to the branch, then merge or open a package for the session counting.

## Blockers and open questions

none

## Decisions taken in this package

- The estimate is the mean over `ESTIMATE_SESSIONS` session ids under seed 42, because mail and FTP sizes vary 5 to 10 fold between sessions. Counts: 100 for ftp, smtp, imap, pop3, 40 for ssh, 20 for the others, 10 for smb2 (time in CI).
- Rounded to three significant digits from 1000 bytes up.
- TLS scenarios count plaintext above TLS.
- Skipped: modules/_template (1280 is right), content (no scenarios), tls (no manifest), strike (MOD-27).

## Pitfalls

- The sandbox disk is full; build with CARGO_TARGET_DIR=/dev/shm/target-MOD-25.
- The worktree guard refuses heredocs that mix with git; write scripts to the scratchpad.

## Verify the current state

```bash
git log --oneline -3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T16:25Z | claude-9c4e1a | claim | none | claimed #494 |
| 2026-11-12T16:50Z | claude-9c4e1a | checkpoint | see git log | estimates, guards, guide, workflow, log entry |
| 2026-11-12T17:10Z | claude-9c4e1a | checkpoint | see git log | PR 518 opened; qa-gate2 mix fails on bytes basis, workflow restored |
| 2026-11-12T17:35Z | claude-9c4e1a | review | 890738d | CI green except the qa-gate2 unit test of QA-05 (outside paths); mix passes with the waiver |
