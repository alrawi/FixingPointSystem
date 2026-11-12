# Handoff MOD-23 The http2 module capture that missed one TLS session

| Field | Value |
|---|---|
| Issue | #478 |
| Branch | `wp/MOD-23` |
| Pull request | #0 |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-ace46d, lease until 2026-11-12T04:27:06Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The job `module (http2)` of run 43077861098 (job 130722417029, pull request #473, which did not touch http2) failed `realism decrypted` with "the capture has 39 TLS sessions, expected 40". It passed on rerun. Find the cause (capture start, a session still open at the stop, or the generator), fix it, and show three consecutive module runs with 40 of 40 TLS sessions. Paths: `modules/http2/`.

## Plan

- [x] 1. Read the failing job log and find which stage counted 39 and which counted 40.
- [x] 2. Reproduce or prove the cause on the sandbox namespace harness.
- [x] 3. Fix inside `modules/http2/` and add a test or a guard.
- [ ] 4. Open the PR, get three green mod-http2 module runs, set ready for review.

## Done and verified

- The log of job 130722417029 (REST): the conformance (`TLS: 40`), encryption (40 of 40 decrypted, 40 JA4) rows of the same capture say 40, only `realism decrypted` says 39 (39 of 39 decrypt). So the capture held 40 connections and ClientHellos.
- The decrypted check counts streams with a ClientHello that plain tshark finds. On 8443 and 8444 that rests on the tshark heuristic, while the encryption check passes `-d tcp.port==P,tls`. Cutting the first ClientHello of `modules/http2/fixtures/http2.pcapng` after 1 to 4 octets gives the exact message (39 TLS sessions, expected 40) and 5 octets or more gives 40; reordering, duplicating, or dropping handshake frames gives 40 (scripts in the scratchpad, `*-MOD-23.py`).
- A reused source port is not the cause: tshark keeps two streams, and the realism runner would have joined them into 39 sessions in the conformance row. The loop saw reuse in 2 of 11 captures at 100 sessions per scenario.
- Fix: `fixtures/tshark-tls-ports.sh` plus `--tshark` in `capture.sh`, and a diagnostic line when the check fails. `cargo test -p fps-module-http2 --test tshark_wrapper` passes 5 tests, clippy clean, `capture.sh` records 40 of 40 locally (the nDPI 4.2 of the sandbox cannot classify HTTP/2, so the run ends with the conformance failure that nDPI 4.2 always gives).
- `plan.py check`, `render --check`, `lint`, `make lint-mod-http2`, `pytest realism/tests/test_decrypted.py` pass.

## Remaining

Three consecutive green runs of `module (http2)` on the pull request, then `fps_swarm.py review`.

## Next action

Open the pull request, push the code, and rerun the `realism` workflow until three runs in a row are green.

## Blockers and open questions

The artifact `realism-http2` of the failed run (id 13882903267) cannot be downloaded from the sandbox (the blob host answers 403 to CONNECT), so what split the ClientHello in that run is not known. A better fix is in `realism/decrypted.py` (outside the paths of this package), see the log entry.

## Decisions taken in this package

- The fix is a wrapper around tshark that capture.sh passes with `--tshark`, because the paths allow only `modules/http2/`. It adds Decode As for the https ports for the capture file only, because the decrypted view keeps the same ports with HTTP/2 in the clear.

## Pitfalls

- The worktree guard refuses `source` and heredocs in a command that runs git. Pass variables inline and write files with the editor tools.
- `pkill` is refused by the sandbox; stop your own background loop by PID.

## Verify the current state

```bash
cargo test -p fps-module-http2 --test tshark_wrapper -- --nocapture
python3 -m pytest realism/tests/test_decrypted.py -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T02:57Z | claude-ace46d | claim | none | claimed #478 |
| 2026-11-12T03:45Z | claude-ace46d | checkpoint | none | cause found and reproduced, fix and tests written |
