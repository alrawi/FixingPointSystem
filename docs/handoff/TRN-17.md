# Handoff TRN-17 TCP stack bench harness and smoltcp behind SessionIo

| Field | Value |
|---|---|
| Issue | #571 |
| Branch | `wp/TRN-17` |
| Pull request | none yet |
| Status | in progress |
| Progress | 83% (5 of 6 plan steps) |
| Current holder | claude-7e8c87, lease until 2026-11-17T00:23:35Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Build `engine/fps-tcp/`, a harness that drives a user space TCP stack behind `SessionIo` and measures sessions per second, throughput, and socket scaling toward 1,000,000 sockets, with smoltcp as the first stack. TRN-18 adds an own stack on the same harness, and TRN-10 compares the results in decision record 0014. Results are committed for the CI runner, and a host test gives the fat-nuc run.

## Plan

- [x] 1. Claim, branch, handoff, and a claim commit.
- [x] 2. Crate skeleton, smoltcp license check with cargo deny.
- [x] 3. The `TcpStack` seam, the smoltcp adapter, and the `SessionIo` stream.
- [x] 4. The three measures (sessions per second, throughput, socket scaling).
- [x] 5. The bench binary with plain argv, tests, and committed CI results.
- [ ] 6. Docs, log entry, pull request, and CI.

## Done and verified

- `cargo test -p fps-tcp` passes: 22 unit tests, 9 adapter tests through `SessionIo` (`tests/smol.rs`), and 5 workload tests (`tests/workloads.rs`).
- `cargo clippy -p fps-tcp --all-targets -- -D warnings` and `cargo fmt -p fps-tcp -- --check` are clean.
- `cargo deny check` (v0.21.0): bans, sources, and advisories pass. Licenses FAIL for `smoltcp 0.14.0` and `managed 0.8.0`, both `0BSD`, which `deny.toml` does not list. Loosening `deny.toml` belongs to the owner under D13 and is outside this package, so the `cargo-deny` job of CI is red until the owner allows `0BSD`.
- `tcp_bench --profile small` on the sandbox (2 vCPU): about 72,000 sessions per second (0 failed, p99 0.65 ms), about 27 Gbit/s with 4 uploads, and 1.55 KB per socket. Results in `engine/fps-tcp/results/sandbox-2vcpu/small.json`.
- `tcp_bench --profile full --workload scale` on the sandbox: 1,000,584 sockets in 64 shards within 35 s, 1.59 KB per socket, 52 ms for one poll with all sockets idle, about 90 probe sessions per second. Results in `engine/fps-tcp/results/sandbox-2vcpu/scale-1m.json`.
- `python3 tools/plan/plan.py lint` passes.

## Remaining

The fat-nuc run (a host test, which the coordinator runs), the license decision of the owner, and CI results from a hosted runner, which need a workflow file outside this package's paths (see the log entry).

## Next action

Wait for CI on the pull request, then set the handoff to ready for review and run `fps_swarm.py review 571 --pr N`.

## Blockers and open questions

The owner must allow the license `0BSD` in `deny.toml` (or add an exception for `smoltcp` and `managed`), or `cargo deny check` fails in CI. Everything else is finished.

## Decisions taken in this package

- The stack seam is a pair (client host and server host) behind one trait, so TRN-18 implements one type and gets all three workloads. A single host trait would need a peer model in the harness, and a pair keeps the link out of the stack under test.
- Sockets count both hosts of a pair, so 1,000,000 sockets are 500,000 sessions. The catalog says sockets, and the harness reports `sockets` as the stack holds them, with the few listening sockets included.
- The scaling workload uses shards, because smoltcp is linear in the sockets of a set. One shard per worker is also how the engine would use it. The shard count is a flag, and the report names it.
- The adapter takes time from an injected `Clock` and seeds the initial sequence numbers from the config, so tests are deterministic. Only the workloads read the wall clock.
- The checksum, MTU, buffer, Nagle, and TIME-WAIT settings are flags, and the defaults model a plain software port (checksums on, MTU 1500, Nagle off, acknowledge at once, TIME-WAIT skipped), which matches the kernel backend defaults where they exist.
- The driver polls after a session acted and every 100 ms otherwise. A 10 ms idle interval made idle shards poll for more than the available time at 250,000 sockets, which distorted the setup rate (measured, then changed).
- The report is an `fps-bench/v1` document of the suite `tcp-stack`, so `bench/compare.py` can compare runs when a baseline exists. No baseline is added here, because baselines belong to QA reviews.
- No workflow is added, because the package paths exclude `.github/workflows/`. The log entry gives the workflow text for the coordinator.

## Pitfalls

- smoltcp panics when a handle is not in its set. The adapter removes a socket only after its session was dropped or aborted, and it aborts nothing twice (`aborted` flag), so a handle stays valid for the life of its session.
- A listening socket serves exactly one SYN. Without the replacement between two packets, a burst larger than the backlog gets resets. `poll_ingress_single` is what makes the replacement possible, because `poll` would consume all queued SYNs first.
- `SocketSet::add` scans from the start for a free slot, so it is linear too. The scaling numbers include it, and sharding is the fix.
- A socket that was aborted keeps its tuple until the reset left, so the reap list removes a closed socket only when `remote_endpoint()` is `None`.
- smoltcp holds TIME-WAIT for ten seconds with no setting, so a rate run would grow the socket set by the session rate times ten without the skip.
- The sandbox shares memory and CPUs with other workers, so runs vary by up to 20 percent.

## Verify the current state

```bash
cargo test -p fps-tcp
cargo clippy -p fps-tcp --all-targets -- -D warnings
cargo run --release -p fps-tcp --example tcp_bench -- --profile small
# expect: all tests pass, no clippy findings, and a report with sessions per second,
# throughput, and three scale steps, with 0 failed sessions and exit code 0
```

## Host test

Run on fat-nuc through the relay. The command needs no sudo, no namespaces, and no change to the host. It builds the release profile in the clone, so the host needs the Rust toolchain that the runner already uses.

| Item | Value |
|---|---|
| Commit | the head of `wp/TRN-17` after CI passed, or `main` after the merge |
| Argv 1 | `cargo run --release -p fps-tcp --example tcp_bench -- --profile full --label fat-nuc --out -` |
| Argv 2 | `cargo run --release -p fps-tcp --example tcp_bench -- --workload rate --threads 4 --label fat-nuc-4-threads --out -` |
| Argv 3 | `cargo run --release -p fps-tcp --example tcp_bench -- --workload throughput --threads 4 --label fat-nuc-4-threads --out -` |
| Sudo | no |
| Timeout | 1800 s for argv 1 (the build takes 1 to 3 minutes and the run about 1 minute on a 2 vCPU host), 600 s for argv 2 and 3 |
| Memory | about 2 GiB for the 1,000,000 sockets of argv 1 |
| Start window | not between 00:50 and 03:30 UTC, as decision D13 says |

The expected result of each argv is exit code 0 and a report on standard output that ends with an `fps-bench/v1` JSON document. Argv 1 prints the session rate with `0 failed`, the throughput, and a scaling table for the steps up to 1,000,000 sockets over 64 shards. If a step exceeds its 300 s budget, the table marks it `over budget`, the run stops there and still exits with 0, and the sockets reached are the result. A nonzero exit code means failed sessions or a harness error, and the message names the cause. The coordinator posts the three reports on the pull request. Nothing is committed from the host.

The fat-nuc result is the host that TRN-10 compares smoltcp and the own stack on, so argv 1 should run again for TRN-18 with the same flags. Add `--checksums off` to a fourth run to see the effect of checksum offload.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16 | claude-7e8c87 | claim | none | Claimed #571 with 6 steps |
| 2026-11-16 | claude-7e8c87 | checkpoint | 5a24511 | The three workloads, the tcp_bench command, tests, and sandbox results |
