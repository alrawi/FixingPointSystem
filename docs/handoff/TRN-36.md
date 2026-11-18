# Handoff TRN-36 An AF_XDP realism job for every first party module

| Field | Value |
|---|---|
| Issue | #682 |
| Branch | `wp/TRN-36` |
| Pull request | none yet |
| Status | in progress |
| Progress | 10% (0 of 5 plan steps) |
| Current holder | claude-worker, lease until 2026-11-18T21:34:51Z |
| Last verified commit | none |

## Goal

Run every first party module through the agent forced onto AF_XDP and score the capture like its kernel run. Generalize `engine/fps-datapath/realism/http1-afxdp.sh` and `make_plan.py` to any module under `modules/`. A module that cannot pass yet goes on a written exclusion list with its reason and the kernel fallback of AGT-29 that covers it. The coordinator gets the exact CI YAML.

## Plan

- [ ] 1. Generic `module-afxdp.sh`, `make_plan.py`, `profiles.yaml`, and tests (no root).
- [ ] 2. Run each module locally as root, fix what lies inside the paths.
- [ ] 3. Exclusion list with causes, results under `engine/fps-datapath/results/`.
- [ ] 4. Transport page, log entry, CI YAML.
- [ ] 5. Checks, pull request, CI.

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Build `fps-agent` (debug) with `CARGO_TARGET_DIR=$PWD/target-TRN-36` and run `sudo engine/fps-datapath/realism/module-afxdp.sh MODULE target-TRN-36/debug/fps-agent OUT` for each module.

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
| 2026-11-18T21:05Z | claude worker | claim | none | claimed #682 |
