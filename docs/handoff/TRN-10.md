# Handoff TRN-10 User space TCP stack decision

| Field | Value |
|---|---|
| Issue | #561 |
| Branch | `wp/TRN-10` |
| Pull request | opened after this commit, see the issue card |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-9d41e7, lease until 2026-11-18T01:16:47Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Propose decision record 0014 for the TCP stack of the AF_XDP backend (TRN-12): compare smoltcp, the own stack of TRN-18, and F-Stack (paper only), with the measurements of TRN-17, TRN-18, and TRN-20, a license review, and the follow-up packages for TRN-12. The record must not alter D1 or a tier target.

## Plan

- [x] 1. Re-run the missing rows in one session in the sandbox and commit the results under `engine/fps-tcp/results/`.
- [x] 2. Write `docs/decisions/0014-user-space-tcp-stack.md` and add it to the decisions index.
- [x] 3. Write the log entry and run the checks.
- [ ] 4. Open the pull request, wait for CI, and move to review.

## Done and verified

- Step 1: `engine/fps-tcp/results/sandbox-2vcpu-trn10/` holds the kernel, own, and smoltcp runs of one session (`host.txt` lists the argv). The kernel row at 1,000,000 sockets is not measured, because the open files hard limit of the sandbox is 20,000.
- Step 2 and 3: record 0014 (status Proposed), the index line, a section in `docs/wiki/Transport.md`, and the log entry. `plan.py check`, `render --check`, and `lint` exit 0. No engine code changed, so no cargo run beyond the benchmark build.

## Remaining

- Step 4. The vetting agent accepts or rejects the record under D13 and the coordinator then changes its status line.

## Next action

Wait for CI on the pull request, then run `fps_swarm.py review 561 --pr N`.

## Host test

No new host test. The fat-nuc run of the three stacks still waits for a Rust toolchain on the relay account. Argv for the relay, no sudo needed for the build, an open files limit above 1,100,000 for the kernel scaling row: `target/release/examples/tcp_bench --stack S --profile full --label fat-nuc --out -` for S in kernel, own, and smoltcp, as in the TRN-17 and TRN-20 handoffs.

## Blockers and open questions

The two open questions for the owner are at the end of record 0014. F-Stack's license file and maintenance state were not confirmed (a fetch of the repository files was not permitted in this session).
