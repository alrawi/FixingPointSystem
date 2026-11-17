# Handoff TRN-11 AF_XDP packet input and output

| Field | Value |
|---|---|
| Issue | #562 |
| Branch | `wp/TRN-11` |
| Pull request | none yet |
| Status | in progress |
| Progress | 83% (5 of 6 plan steps) |
| Current holder | claude-7c41e9, lease until 2026-11-17T00:23:48Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Build the crate `engine/fps-backend-afxdp` with the UMEM, fill, completion, receive, and transmit rings, batching per queue, zero copy mode with a fallback to copy mode, per queue workers, and a published frame trait with batched receive and transmit per queue. Defaults come from spike S3. Acceptance: AF_XDP on a veth pair in the namespace harness passes frames both ways in CI, falls back to copy mode, and reads the bind mode from the socket. The KVM rate test is TRN-19 and not part of this package.

## Plan

- [x] 1. Claim, handoff file, and crate skeleton with the frame trait and an in memory fake
- [x] 2. UMEM, the four rings, and the `sys` module for the socket and mmap calls
- [x] 3. XDP program loader and bind with zero copy, copy fallback, and bind mode read back
- [x] 4. Per queue workers and batched receive and transmit
- [x] 5. Veth test in the namespace harness (no Host test needed, see Decisions)
- [ ] 6. Wiki ingest, log entry, checks, pull request, and CI

## Done and verified

- Steps 1 to 5. `cargo test -p fps-backend-afxdp` passes: 27 unit tests and the veth integration test. `cargo clippy -p fps-backend-afxdp --all-targets -- -D warnings` and `cargo fmt -p fps-backend-afxdp -- --check` are clean.
- The veth test (tests/veth.rs) ran as root in this sandbox (Linux 6.18): 100,000 frames per queue crossed `eth0` and `cli0` of the QA-01 harness and came back with matching payloads, once on one queue and once on two queues with one worker per queue on a veth pair with two queues. Zero copy bind failed with errno 95, Auto and Copy bound in copy mode, the mode read from the socket was copy, and a second port on the same interface failed with EBUSY. Eight repeated runs with 20,000 frames per queue passed.
- The skip path ran as the user nobody (skips) and with CI=true (fails), and the sudo re-exec command line ran as root.

## Remaining

Wiki ingest (Transport page), log entry with Shared page notes, crate README, checks of AGENTS.md, pull request, and CI.

## Next action

Write the Transport wiki section, the log entry, and the README, then run the AGENTS.md checks and open the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- The frame trait `FrameIo` lives in this crate, because the catalog paths of TRN-11 exclude `engine/fps-io`. A later package can move it to `fps-io` if a DPDK backend needs it, which is an additive change.
- Receive and send lend memory through callbacks (`recv_batch`, `send_batch_with`) so a batch costs no allocation. `send_batch` is the copying convenience.
- One socket and one UMEM per queue, so workers share nothing. The UMEM holds `2 * ring_size` frames, half for receive and half for transmit.
- The crate depends only on `libc` and `thiserror`, and declares the AF_XDP and BPF structures itself in `sys.rs`. The XDP program is six hand written BPF instructions that call `bpf_redirect_map` on the queue index and pass the frame to the stack when no socket serves the queue. It attaches through a BPF link, which the kernel detaches when the last queue closes, so no program outlives the process.
- Bind mode `Auto` leaves the choice to the kernel, which tries zero copy and falls back to copy mode. The mode comes from `getsockopt(XDP_OPTIONS)` and not from the request.
- Defaults from S3 are batch 32 (the burst of the S3 `testpmd` runs), one worker per queue, automatic mode because zero copy did not beat copy beyond the host noise, and 2048 byte frames. Ring size 2048 is not an S3 result.
- The veth test starts itself again under `sudo -n` when it is not root, so the `rust` job of `ci.yml` runs it without a workflow change. `CI=true` or `FPS_REQUIRE_AFXDP=1` turn a skip into a failure.
- No Host test is needed. The acceptance runs on a veth pair in CI, and the KVM rate test is TRN-19.

## Pitfalls

- The kernel releases the queue of a closed AF_XDP socket in a work queue, so a bind right after a close can fail with EBUSY for a few milliseconds. The test retries for up to 5 seconds.
- `recv_batch` must refresh the cached consumer index of the fill ring before it pushes recycled frames, or the ring looks full after one pass and the queue stalls after `ring_size` frames. The first run stalled at exactly 256 frames.
- Copy mode sends inside `sendto`, at most 32 frames per call, and returns EAGAIN when more are pending, so `kick_tx` repeats the call.
- A veth pair refuses zero copy with errno 95 (EOPNOTSUPP), and its native XDP mode accepts the program. The test keeps the harness MTU of 1500. A larger MTU may make the veth refuse native XDP, which the code handles only for EOPNOTSUPP, and no test here covers it.

## Verify the current state

```bash
cargo test -p fps-backend-afxdp
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16T22:55Z | claude-7c41e9 | claim | none | Claimed #562 with card 6931824639 |
| 2026-11-16T23:10Z | claude-7c41e9 | checkpoint | see git log | Crate, rings, XDP loader, workers, and the veth test pass locally |
