# Handoff TRN-36 An AF_XDP realism job for every first party module

| Field | Value |
|---|---|
| Issue | #682 |
| Branch | `wp/TRN-36` |
| Pull request | none yet |
| Status | in progress (checks and pull request) |
| Progress | 90% (4 of 5 plan steps) |
| Current holder | claude-648128, lease until 2026-11-18T22:01:25Z |
| Last verified commit | none |

## Goal

Run every first party module through the agent forced onto AF_XDP and score the capture like its kernel run. Generalize `engine/fps-datapath/realism/http1-afxdp.sh` and `make_plan.py` to any module under `modules/`. A module that cannot pass yet goes on a written exclusion list with its reason and the kernel fallback of AGT-29 that covers it. The coordinator gets the exact CI YAML.

## Plan

- [x] 1. Generic `module-afxdp.sh`, `make_plan.py`, `profiles.yaml`, and tests (no root).
- [x] 2. Run each module locally as root, fix what lies inside the paths.
- [x] 3. Exclusion list with causes, results under `engine/fps-datapath/results/`.
- [x] 4. Transport page, log entry, CI YAML.
- [ ] 5. Checks, pull request, CI.

## Done and verified

- All 15 modules with scenarios pass locally as root (tshark and Zeek parsers, nDPI 4.14, the extra checks of each kernel run), in three consecutive sweeps. Results are in `engine/fps-datapath/results/sandbox-2vcpu-trn36/`.
- Excluded: `dns/tcp_fallback`, `hls/vod_segments`, `strike/conn_churn`, with causes and fixes in the log entry and on the Transport page.
- `python3 -m pytest -q engine/fps-datapath/realism`: 47 passed.

## Remaining

Plan step 5: checks, pull request, CI, review.

## Next action

Wait for CI. Run `fps_swarm.py review 682 --pr N` when green. (Local runs: build `fps-agent` (debug) with `CARGO_TARGET_DIR=$PWD/target-TRN-36` and run `sudo engine/fps-datapath/realism/module-afxdp.sh MODULE target-TRN-36/debug/fps-agent OUT` for each module.)

## Blockers and open questions

none

## Decisions taken in this package

- A plan listens once for each port of a server host, and most modules run all scenarios on one port, so the scenarios run in rounds (one agent run per round, all in one capture) and the labels of the kernel run keep their ports. TRN-12 had moved the plain http1 scenarios to other ports instead, which a tshark dissector that works by port would not follow.

## Pitfalls

- Each round needs its own client addresses, because the own stack numbers client ports upward from 10000 in every run, and a 5-tuple that comes up twice merges two sessions.

## Verify the current state

```bash
python3 -m pytest -q engine/fps-datapath/realism
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T21:05Z | claude-648128 | claim | 88b4119 | claimed #682 |
| 2026-11-18T21:40Z | claude-648128 | push | ab86316 | generic runner, profiles, tests |
| 2026-11-18T22:00Z | claude-648128 | local runs | none | 15 modules pass, 3 scenarios excluded |
