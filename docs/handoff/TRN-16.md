# Handoff TRN-16 Per profile captures scored with the transport check

| Field | Value |
|---|---|
| Issue | #405 |
| Branch | `wp/TRN-16` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (4 of 4 plan steps, CI pending) |
| Current holder | claude-80bdc1, lease until 2026-11-10T18:55:30Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`tests/netns/profiles.sh` records one client capture per OS profile and nothing scored it. It now scores each capture with `python3 -m realism score --os-profile NAME` (strict only for linux6), so that a capture recorded with the wrong profile fails, and the window scale 10 of the harness fixtures is explained.

## Plan

- [x] 1. Reproduce the harness window scale locally in a namespace and explain it.
- [x] 2. Add the scoring step to profiles.sh and the wrong profile negative scores.
- [x] 3. Run the capture test locally (root and namespaces work in this sandbox) and the lint and unit checks.
- [x] 4. Log entry with the shared page notes, PR, CI.

## Done and verified

- Fresh harness without a profile: `tcp_rmem` is `4096 131072 33554432` in both namespaces and the SYN and SYN-ACK show window scale 10. The inherited host `tcp_rmem` is the cause, not `net.core.rmem_max`.
- `FPS_PROBE=target-TRN-16/debug/examples/os_profile_probe engine/fps-backend-kernel/tests/netns/profiles.sh` as root: all three profiles pass the handshake check and the realism transport check, the three rotated wrong profile scores and the baseline as linux6 fail as required.
- With `ttl: 64` temporarily in `windows11.yaml` (restored) the windows11 capture fails the transport check.
- `make lint-trn test-trn`, `plan.py check`, `render --check`, `lint`: pass.

## Remaining

CI of the PR, and the coordinator adds `libndpi-bin python3-yaml` to the install step of the `capture` job (see the log entry).

## Next action

Poll the PR checks and fix failures inside `engine/fps-backend-kernel/`.

## Blockers and open questions

The capture job of `trn.yml` fails until the coordinator adds `libndpi-bin python3-yaml` to its apt install step. Workflow files are the coordinator's.

## Decisions taken in this package

- The test reads the `transport_behavior` entry of the report, not the runner's exit code, because the conformance check always runs and the probe protocol is Unknown to nDPI.
- Labels are a JSON file written by the test, so no PyYAML is needed for labels; PyYAML is needed only for the profile files.
- Only linux6 is strict; windows11 and macos14 are not (option order, and the SYN window of macos14).
- The fixtures keep window scale 10; no fix of the harness, because the sessions with a profile write `tcp_rmem` themselves.

## Pitfalls

- The sandbox guard refuses compound commands that mention git; split them.
- `python3 -m realism score` exits 1 for the probe sessions because of conformance, so never use its exit code as the verdict.

## Verify the current state

```bash
cargo build -p fps-backend-kernel --example os_profile_probe
sudo FPS_PROBE=$PWD/target/debug/examples/os_profile_probe engine/fps-backend-kernel/tests/netns/profiles.sh
make lint-trn test-trn
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T17:26Z | claude-80bdc1 | claim | none | claimed #405 |
| 2026-11-10T17:40Z | claude-80bdc1 | checkpoint | none | scoring, negatives, docs, log entry done locally |
