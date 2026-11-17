# Handoff TRN-20 smoltcp on the TCP stack bench harness

| Field | Value |
|---|---|
| Issue | #594 |
| Branch | `wp/TRN-20` |
| Pull request | #0 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-1cd665, lease until 2026-11-18T00:39:00Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Bring the smoltcp adapter that the first TRN-17 worker built (preserved on `wp/TRN-20`) onto the merged TRN-17 harness, next to the kernel reference stack and the own stack of TRN-18, all behind the `TcpStack` seam, and report the same three measures for smoltcp: sessions per second, memory per socket with the idle poll cost, and throughput. The owner allowed the license 0BSD on 2026-11-17, so `cargo deny check` accepts `smoltcp 0.14.0` and `managed 0.8.0`.

## Plan

- [x] 1. Merge `origin/main` into `wp/TRN-20` with an additive merge commit (no force push), restore `src/smol.rs`, `src/link.rs`, `tests/smol.rs`, the dependency, and the `tcp_bench` flags.
- [x] 2. Add `TcpStack::notes` to the smoltcp adapter, wire `--stack smoltcp` into `cli.rs`, and keep `cargo test`, `clippy`, and `fmt` green.
- [x] 3. Measure smoltcp with the commands of TRN-17 and TRN-18 and commit the results next to the kernel and own results.
- [x] 4. Update the README, the Transport page, the log entry, and this handoff, with the fat-nuc host test.
- [ ] 5. Open the pull request, wait for CI, and move the issue to review.

## Done and verified

- Step 1 and 2: `cargo test -p fps-tcp`, `cargo clippy -p fps-tcp --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `cargo deny check licenses` pass.

- Step 3: results in `engine/fps-tcp/results/sandbox-2vcpu-trn20/` (see `host.txt` for the commands). smoltcp, 32 concurrent: 59,000 to 66,000 sessions per second; 4 uploads 28.1 Gbit/s; 1,000,584 sockets in 64 shards at 1.59 KB per socket; idle poll 24 ns per socket in one set of 16,016 and 54 ns per socket over 1,000,584; probe rate 4,800 per second at 1,016 sockets, 60 at 1,000,584.
- Step 4: README, Transport page (new sections), log entry. `plan.py check`, `render --check`, and `lint` pass.

## Remaining

- Step 5: open the pull request, wait for CI, move the issue to review.
- The fat-nuc run waits for an owner action, see Host test.

## Next action

Open the pull request, wait for CI, fix failures inside `engine/fps-tcp/`, then run `fps_swarm.py review 594 --pr N`.

## Blockers and open questions

The fat-nuc run is not possible yet: the owner's relay account on fat-nuc has no Rust toolchain. Owner action: install a Rust toolchain (rustup, stable) for the relay account on fat-nuc. Everything else of the package does not wait for it, so the package does not block on it.

## Decisions taken in this package

- The smoltcp flags carry the prefix `--smol-` (`--smol-mtu`, `--smol-checksums`, `--smol-buffer-kib`, `--smol-time-wait`, `--smol-idle-poll-ms`), like the `--own-` flags of TRN-18, because the flags `--mtu`, `--checksums`, `--time-wait`, and `--idle-poll-ms` are a test of the merged harness (`flags_of_removed_stacks_are_unknown`) and would collide with the other stacks.
- The branch `wp/TRN-20` keeps its history. The merge of `origin/main` takes the tree of main and adds the smoltcp files on top, so the push is a fast forward.

## Pitfalls

- A worktree guard refuses `source` and compound git commands. Use `export` and plain commands.

## Host test

Waits for the owner (Rust toolchain on the relay account of fat-nuc). When it exists, run with the same flags as TRN-17 and TRN-18 so that TRN-10 compares one host. No sudo, no namespaces, no change to the host.

| Item | Value |
|---|---|
| Commit | the head of `wp/TRN-20` after CI passed, or `main` after the merge |
| Argv 1 | `cargo run --release -p fps-tcp --example tcp_bench -- --stack smoltcp --profile full --label fat-nuc-smoltcp --out -` |
| Argv 2 | `cargo run --release -p fps-tcp --example tcp_bench -- --stack smoltcp --workload rate --threads 4 --label fat-nuc-smoltcp-4-threads --out -` |
| Argv 3 | `cargo run --release -p fps-tcp --example tcp_bench -- --stack smoltcp --workload throughput --threads 4 --label fat-nuc-smoltcp-4-threads --out -` |
| Sudo | no |
| Timeout | 1800 s for argv 1 (the build takes 1 to 3 minutes), 600 s for argv 2 and 3 |
| Expected | exit code 0 and an `fps-bench/v1` document with the metrics `smoltcp.rate.*`, `smoltcp.throughput.*`, and `smoltcp.scale.*`. The scaling run reaches 1,000,000 sockets with `--shards 64`, see the results of the sandbox |

## Verify the current state

```bash
cargo test -p fps-tcp
cargo clippy -p fps-tcp --all-targets -- -D warnings
cargo run --release -p fps-tcp --example tcp_bench -- --stack smoltcp --profile small --out -
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-17 | claude-1cd665 | claim | none | Claimed #594, merged origin/main into wp/TRN-20 |
