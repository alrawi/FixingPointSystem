# Handoff TRN-12 User space TCP over AF_XDP, end to end

| Field | Value |
|---|---|
| Issue | #563 |
| Branch | `wp/TRN-12` |
| Pull request | #672 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-7c41e9, lease until 2026-11-18T17:43:15Z |
| Last verified commit | e56b445 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The http1 module passes realism CI on the AF_XDP backend: the agent forced onto AF_XDP runs the http1 scenarios over a veth pair, and the capture scores like the kernel run (conformance, parser errors, encryption). The coordinator adds the CI job.

## Plan

- [x] 1. Read the code, run the agent on a veth pair locally
- [x] 2. Script that records the http1 run on AF_XDP (`engine/fps-datapath/realism/`)
- [x] 3. Fix what the run finds inside the paths (nothing; one gap in the agent, described in the log entry)
- [x] 4. Log entry, Transport page, local checks
- [x] 5. Pull request and CI (all 17 workflows green on e56b445)

## Done and verified

- `sudo engine/fps-datapath/realism/http1-afxdp.sh target-TRN-12/debug/fps-agent OUT` with the stock agent: 199 of 199 sessions pass, conformance and parser errors pass, the encryption check fails for lack of keys (the script notices the agent has no `run --keylog` and leaves the `tls` labels out, exit 0).
- The same run with the agent change of the log entry (scratch copy outside the repo): conformance, parser errors, encryption pass, 68 of 68 TLS sessions decrypt, `realism decrypted` passes.
- `python3 -m pytest -q engine/fps-datapath/realism` (4 passed), `shellcheck`, `ruff`, `mypy --strict` on the new files, `plan.py check`, `render --check`, `lint`, `build_log.py --check`.

## Remaining

The coordinator adds the workflow job (see the log entry, Shared page notes) and the agent change for `run --keylog`. The PR runs no job for the new script.

## Next action

Coordinator review and merge.

## Blockers and open questions

none. The key log of a standalone agent run is an agent change outside the paths; the exact diff is in the log entry.

## Decisions taken in this package

- One agent process on a veth pair with a bridge as the tap: the agent starts AF_XDP only with `--side both`, AF_XDP takes the frames before any tap, and the harness router needs next hop resolution in the agent (TRN-35, AGT-26).
- The plain scenarios get one port each (8080 to 8085) because two mix entries cannot listen on one port of one server host.
- No `--scenario` structure check: it fails on the committed kernel capture too, and the kernel run does not use it.

## Pitfalls

- `grep -q` on the output of the agent under `set -o pipefail` ends the pipe early and fails the pipeline with SIGPIPE; the script reads the usage text into a variable instead.
- A tcpdump on either veth end of an AF_XDP port sees nothing.
- The sandbox has no IPv6, so the `sysctl` that turns it off must tolerate a missing file.

## Verify the current state

```bash
sudo engine/fps-datapath/realism/http1-afxdp.sh target/debug/fps-agent /tmp/http1-afxdp   # exit 0, three checks pass
python3 -m pytest -q engine/fps-datapath/realism                                         # 4 passed
```

## Host test

Not needed: the run needs only root, network namespaces, and veth, which hosted runners and the sandbox have. Argv: `engine/fps-datapath/realism/http1-afxdp.sh PATH_TO_FPS_AGENT OUTPUT_DIR`, sudo, timeout 120 seconds, expected exit 0 and the line `realism decrypted: 68 of 68 TLS sessions decrypt` (the count varies) when the agent has `--keylog`.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T17:21Z | claude-7c41e9 | claim | none | Claimed #563 |
| 2026-11-18T17:33Z | claude-7c41e9 | checkpoint | 16b6f56 | The script and the plan generator |
| 2026-11-18T18:10Z | claude-7c41e9 | ready for review | e56b445 | PR #672, CI green; rerun on merged main (TRN-35) passed |
