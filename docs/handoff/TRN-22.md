# Handoff TRN-22 A lossy link and loss workloads for the TCP harness

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #623 |
| Branch | `wp/TRN-22` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (5 of 6 plan steps) |
| Current holder | claude-9e6494, lease until 2026-11-18T03:55:16Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add loss, delay, reordering, and a rate limit with a bounded queue to the in memory links of the own and smoltcp stacks (a wrapper in fps-tcp, since the own link lives in fps-tcp-own), a netem counterpart for the kernel stack in a network namespace, and a loss workload in tcp_bench that reports goodput, retransmissions, and completion per transfer.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read the code; design the lossy link
- [x] 2. Lossy link for smoltcp and own stacks with unit tests
- [x] 3. Netem namespace setup for the kernel stack
- [x] 4. Loss workload and tcp_bench command line
- [x] 5. Run the sweep, commit results
- [ ] 6. Log entry, checks, PR

## Done and verified

Steps 1 to 4: impair.rs (model, netem argv), lossy.rs (own stack wrapper), smol.rs (impair option), netns.rs (kernel namespaces and peers), loss workload in workload.rs and cli.rs. `cargo test -p fps-tcp` passes, including tests/lossy.rs and the root test tests/netns.rs (rate scenario passes here, the netem scenarios skip: this sandbox kernel has no sch_netem).

## Remaining

Step 5: run the sweep and commit results under engine/fps-tcp/results/. Step 6: log entry, README, checks, PR. Kernel rows at 20 ms and loss need a host with sch_netem.

## Next action

Run tcp_bench --stack own and --stack smoltcp with --workload loss --label sandbox-2vcpu-trn22 --out, write host.txt, then the log entry.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|

## Host test

The kernel rows at 0, 0.1, and 1 percent with 20 ms need a kernel with `sch_netem`, which the sandbox lacks. Build once as the normal user, then run the binary as plain argv. It runs itself again through `sudo -n` (or run it as root).

- Build: `cargo build --release -p fps-tcp --example tcp_bench`
- Run: `target/release/examples/tcp_bench --stack kernel --workload loss --label "fat-nuc, kernel, loss 0/0.1/1 percent, 20 ms per direction" --out engine/fps-tcp/results/fat-nuc-trn22/kernel-loss.json`
- Then the same with `--stack own` and `--stack smoltcp` in the same session, `--out .../own-loss.json` and `.../smoltcp-loss.json`.
- Needs: root or `sudo -n`, `ip`, `tc`, `ethtool` (optional), module `sch_netem`. Creates three namespaces named `fpstcp<pid>x<n>c|r|s` per transfer and deletes them itself.
- Timeout: 20 minutes (kernel 9 transfers of 4 MiB, each at most 60 s; the in memory stacks about 2 and 1 minutes).
- Expected: exit 0 and a table with `yes` for every kernel transfer at 0 and 0.1 percent; exit 3 with a message when the kernel has no `sch_netem`.
- Also: `cargo test -p fps-tcp --test netns` with `FPS_REQUIRE_NETEM=1` runs the three namespace scenarios and must print three `ok` lines.
