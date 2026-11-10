# Handoff MOD-21 Parser errors on live http1 captures

| Field | Value |
|---|---|
| Issue | #361 |
| Branch | `wp/MOD-21` |
| Pull request | #367 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-4e7c21, lease until 2026-11-10T06:27:13Z |
| Last verified commit | 2557032 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The realism job `module (http1)` records a fresh capture in the namespace harness and failed intermittently with parser errors on plain HTTP sessions. Find the sessions and the cause, fix it in the scenario, the module function, or the capture script without weakening the parser error check, and show that the job passes 20 times in a row.

## Plan

- [x] 1. Read the failed jobs and the realism code, find the cause
- [x] 2. Fix the peer and guard the capture script, check locally
- [x] 3. Temporary CI job with 20 runners, 20 passes in a row
- [x] 4. Remove the temporary job, pull request, CI

## Done and verified

- Cause. The failed jobs 129559371317, 129569328952 (and a third, 129579545446 on main, run 42740105979) all show `conformance http1` over fewer sessions than the 120 that ran (116, 116, 118) and as many `parser_errors` as lost sessions (4, 4, 2); the https scenarios were intact. The session table of the realism runner keys sessions by 5-tuple, so a client source port that the kernel picked a second time for destination port 8080 merges two sessions. 120 plain sessions go to one destination and one port, which makes a repeat likely. The artifacts of the failed runs cannot be downloaded from this sandbox (the blob host is blocked), so the exact Zeek weird is not seen.
- Local reproduction. The harness runs here (tshark 4.2.2, ip, tcpdump, nDPI; no Zeek). With `net.ipv4.ip_local_port_range` narrowed to 161 ports in the client namespace, the capture holds repeats of a 5-tuple that tshark marks `TCP Port numbers reused` (a note, no error), and the session count falls below 200. Without narrowing, 8 of 8 captures pass with tshark alone, so the failure needs the Zeek check or a kernel that repeats ports.
- Fix. `fixtures/` and `native/examples/fixture_peer.rs`: the client peer gives every session its own source port (40000 on, `IP_LOCAL_PORT_RANGE` of one port), so no 5-tuple repeats in a capture. `capture.sh` fails with the cause when the capture holds fewer sessions than the peers ran. The parser error check and `expected.yaml` are unchanged.
- Local run after the fix: `realism/ci/module-realism.sh http1` with tshark passed 3 of 3 times, and clippy and `cargo fmt --check` are clean.

- Acceptance. The temporary workflow `tmp-mod-21` (run 42759200032, commit 2b3756e) ran the job on 20 runners, and each runner recorded and scored three captures in a row with tshark 4.2.2 and Zeek 9.0.1. All 20 jobs passed, which is 60 fresh captures with 120 of 120 plain sessions and no parser error. The workflow is removed in the next commit.

## Remaining

None.

## Next action

Wait for the CI of the pull request, then run `fps_swarm.py review`.

## Blockers and open questions

none

## Decisions taken in this package

- Fix in the peer and not in the engine connector, because the paths of this package are `modules/http1/`. The other module peers (dns, ftp, smtp, imap, pop3) use the same kernel connector and one destination port per scenario, so they can show the same fault with more sessions per port. See the log entry.
- Per session source ports come from `IP_LOCAL_PORT_RANGE` (Linux 6.3 or later, which the `ubuntu-24.04` runner has), and not from a bind to an explicit port, because the connector binds port 0 itself.

## Pitfalls

- The artifact download of a failed run (`gh api .../artifacts/<id>/zip`) is refused by the agent proxy here. Download it from a machine with access, or read `http1.realism.json` in the job artifact `realism-http1`.
- A one-port range outside `net.ipv4.ip_local_port_range` makes every connect fail, so the narrowed test must keep ports 40000 to 59999 inside the range.

## Verify the current state

```bash
sudo FPS_REALISM_PARSERS=tshark realism/ci/module-realism.sh http1 /tmp/http1-out   # all checks pass
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T05:00Z | claude-4e7c21 | claim | none | Claimed #361 with 4 plan steps |
| 2026-11-10T06:05Z | claude-4e7c21 | review | 2557032 | PR #367 open, 17 checks reported, all green except two still running (Build, boot, and run; gate), module (http1) and negative-control green |
