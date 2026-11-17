# Handoff TRN-17 TCP stack bench harness with a kernel reference stack

| Field | Value |
|---|---|
| Issue | #571 |
| Branch | `wp/TRN-17` |
| Pull request | #586 |
| Status | ready for review |
| Progress | 100% |
| Current holder | claude-7a1c3e |
| Last verified commit | 5a73c66 (CI green: `rust`, `licenses`, and all 61 checks), then merged with main at b60f335 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Build `engine/fps-tcp/`, a harness that drives a TCP stack behind `SessionIo` and measures sessions per second, throughput, and socket scaling toward 1,000,000 sockets. The kernel loopback TCP is the reference stack. TRN-18 adds an own stack on the same harness, TRN-20 adds smoltcp, and TRN-10 compares the results in decision record 0014. Results are committed for the sandbox, and a host test gives the fat-nuc run.

The package was re-scoped under D13 on 2026-11-16 (catalog text on `coord/batch-36`, PR #587). `cargo deny` rejects `smoltcp 0.14.0` and `managed 0.8.0`, which carry the license 0BSD that `deny.toml` does not list, and loosening `deny.toml` stays with the owner. So the harness is independent of smoltcp, and the smoltcp adapter is TRN-20.

## Plan

- [x] 1. Claim, branch, handoff, and a claim commit.
- [x] 2. Crate skeleton.
- [x] 3. The `TcpStack` seam, the workloads, the clock, and the host facts (first worker).
- [x] 4. D13 re-scope: the smoltcp work preserved on `wp/TRN-20`, smoltcp and managed removed from `Cargo.toml` and `Cargo.lock`.
- [x] 5. The kernel loopback reference stack, the bench binary with plain argv, tests, and sandbox results of the reference stack.
- [x] 6. Docs, log entry, pull request text, and CI.

## Done and verified

- `wp/TRN-20` holds the complete smoltcp adapter on top of the harness (commit `58ab16c`, the head of `wp/TRN-17` before the re-scope), pushed with no pull request. Its draft numbers are 1,000,584 sockets at 1.59 KB per socket, and an idle poll of 15 to 52 ns per socket.
- `engine/fps-tcp/Cargo.toml` and `Cargo.lock` hold no `smoltcp` and no `managed` package. `Cargo.lock` lists no `smoltcp`, `managed`, `heapless`, `hash32`, `bitflags 1.3.2`, or `defmt`.
- `cargo deny check` (v0.21.0) passes: advisories ok, bans ok, licenses ok, sources ok. It prints only the usual duplicate version warnings.
- `cargo test -p fps-tcp` passes: 21 unit tests, 8 reference stack tests through `SessionIo` (`tests/kernel.rs`), and 5 workload tests (`tests/workloads.rs`).
- `cargo clippy -p fps-tcp --all-targets -- -D warnings` and `cargo fmt -p fps-tcp -- --check` are clean.
- `tcp_bench --profile small` on the sandbox (2 vCPU, shared): 14,000 to 35,000 sessions per second between runs (0 failed), about 20 Gbit/s with 4 uploads, and 0.35 to 0.5 KB of user space memory per socket. Results in `engine/fps-tcp/results/sandbox-2vcpu/kernel-small.json`.
- `tcp_bench --profile full --workload scale` on the sandbox: 19,815 sockets, where the open files limit of 20,000 refused the next socket, and the run recorded that as a `stopped` line. Results in `engine/fps-tcp/results/sandbox-2vcpu/kernel-scale.json`. The sandbox cannot show 1,000,000 sockets with a kernel stack.
- These are results of the reference stack from the agent sandbox, labelled as such in `host.txt` and in the report labels. They are not CI runner results and not fat-nuc results.

## Remaining

The fat-nuc run of the reference stack (a host test, which the coordinator runs), and CI results from a hosted runner, which need a workflow file outside this package's paths (see the log entry). The smoltcp adapter and its results are TRN-20.

## Next action

The coordinator reviews and merges PR #586, then runs the host test.

## Blockers and open questions

None for TRN-17. The host test needs `cargo` on the `PATH` of the relay account. TRN-20 waits for the owner to allow 0BSD in `deny.toml`, or to allow `smoltcp` and `managed` by exception.

## Decisions taken in this package

- The stack seam is a pair (client host and server host) behind one trait, so TRN-18 and TRN-20 implement one type each and get all three workloads. A single host trait would need a peer model in the harness, and a pair keeps the link out of the stack under test.
- Sockets count both hosts of a pair, so 1,000,000 sockets are 500,000 sessions. The catalog says sockets, and the harness reports `sockets` as the stack holds them, with the few listening sockets included.
- The scaling workload uses shards, because a user space stack such as smoltcp is linear in the sockets of a set. One shard per worker is also how the engine would use it. The shard count is a flag, and the report names it.
- Each stack lists the facts about itself and its link in `TcpStack::notes`, and the report carries them, so a reader of a kernel run is not told about an in memory link.
- The kernel stack maps each logical port of the harness to an ephemeral loopback port, so the shards of a scaling run never collide on a real port, and a run does not need a free fixed port on the host.
- A stack that refuses a socket ends the scaling run with the steps measured so far and a `stopped` line, in place of an error that loses them. The kernel stack reaches this at the open files limit.
- At most 20,000 sessions share one server port in the scale workload, below the 28,000 source ports of a Linux client for one server address and port.
- The report is an `fps-bench/v1` document of the suite `tcp-stack`, so `bench/compare.py` can compare runs when a baseline exists. No baseline is added here, because baselines belong to QA reviews.
- The `Clock` stays in the harness for the stacks that read their time from it (TRN-18, TRN-20), though the kernel stack does not use it.
- No workflow is added, because the package paths exclude `.github/workflows/`. The log entry gives the workflow text for the coordinator.

## Pitfalls

- A kernel reset arrives after the bytes that were already delivered, so a reader gets the data and then `ConnectionReset`. A test that expects the reset on the first read fails on loopback.
- `TcpStack::listen` of the kernel stack registers the socket with the tokio reactor, so it must run inside a runtime built with `enable_io`.
- Kernel socket memory is not in the resident set, so `bytes_per_socket` is a user space number only. Do not compare it with a user space stack as total memory.
- The idle poll of the kernel stack is zero by construction, which the report says in its notes.
- Every socket is a descriptor. The scaling run needs an open files limit above 1,000,100 for 1,000,000 sockets, and `fs.nr_open` must allow it.
- The sandbox shares memory and CPUs with other workers, so session rates vary by up to a factor of two.

## Verify the current state

```bash
cargo test -p fps-tcp
cargo clippy -p fps-tcp --all-targets -- -D warnings
cargo deny check
cargo run --release -p fps-tcp --example tcp_bench -- --profile small
# expect: all tests pass, no clippy findings, deny passes, and a report with sessions per second,
# throughput, and three scale steps, with 0 failed sessions and exit code 0
```

## Host test

Run on fat-nuc through the relay. The command needs no sudo, no namespaces, and no change to the host. It builds the release profile in the clone, so the host needs the Rust toolchain that the runner already uses. The account of the relay currently lacks `cargo` on `PATH`, so this host test cannot run until the relay account has `cargo` (for example through its `~/.cargo/bin` on `PATH`) or the coordinator runs a prebuilt binary. The commands below are the final form, with plain argv.

| Item | Value |
|---|---|
| Commit | the head of `wp/TRN-17` after CI passed, or `main` after the merge |
| Argv 1 | `cargo run --release -p fps-tcp --example tcp_bench -- --profile full --label fat-nuc-kernel --out -` |
| Argv 2 | `cargo run --release -p fps-tcp --example tcp_bench -- --workload rate --threads 4 --label fat-nuc-kernel-4-threads --out -` |
| Argv 3 | `cargo run --release -p fps-tcp --example tcp_bench -- --workload throughput --threads 4 --label fat-nuc-kernel-4-threads --out -` |
| Sudo | no |
| Timeout | 1800 s for argv 1 (the build takes 1 to 3 minutes), 600 s for argv 2 and 3 |
| Memory | a few GiB when the open files limit allows 1,000,000 sockets, because each kernel socket also holds kernel memory |
| Start window | not between 00:50 and 03:30 UTC, as decision D13 says |

The expected result of each argv is exit code 0 and a report on standard output that ends with an `fps-bench/v1` JSON document. Argv 1 prints the session rate with `0 failed`, the throughput, and a scaling table. The scaling table ends at the open files limit of the relay account, which the report lists as `open_files_soft_limit`, and the `stopped` line names the refused socket. That is a valid result, and it only reaches 1,000,000 sockets when the limit is above 1,000,100. A nonzero exit code means failed sessions or a harness error, and the message names the cause. The coordinator posts the three reports on the pull request. Nothing is committed from the host.

The fat-nuc result is the host that TRN-10 compares the stacks on, so argv 1 should run again for TRN-18 and TRN-20 with the same flags.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16 | claude-7e8c87 | claim | none | Claimed #571 with 6 steps |
| 2026-11-16 | claude-7e8c87 | checkpoint | 5a24511 | The three workloads, the tcp_bench command, tests, and sandbox results (smoltcp) |
| 2026-11-17 | claude-7e8c87 | block | ed0d90a | PR #586 open. Blocked on the owner allowing 0BSD in deny.toml |
| 2026-11-16 | claude-7a1c3e | takeover | 58ab16c | Re-scoped under D13. The smoltcp work is preserved on `wp/TRN-20` |
| 2026-11-16 | claude-7a1c3e | checkpoint | none | Kernel reference stack, smoltcp removed from the lock file, cargo deny passes |
| 2026-11-16 | claude-7a1c3e | review | b60f335 | CI green on 5a73c66, main merged, ready for review |
