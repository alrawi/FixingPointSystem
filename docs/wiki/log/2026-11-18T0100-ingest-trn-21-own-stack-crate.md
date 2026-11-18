## [2026-11-18] ingest | TRN-21 the own TCP stack in a crate of its own

TRN-21 moved the own stack of TRN-18 out of `fps-tcp` into the new crate `engine/fps-tcp-own` (decision record 0014, item 2).

- `fps-tcp-own` holds `host.rs`, `wire.rs`, the stack and its sessions (`lib.rs`), the clocks (`clock.rs`), and the component tests of the stack (`tests/own.rs`). `cargo tree -p fps-tcp-own` lists no smoltcp. The dependencies are `fps-backend-afxdp`, `fps-io`, `thiserror`, and `tokio`.
- `OwnStack` now has the methods `listen`, `connect`, `accept`, `drive`, and `stats` of its own (with the type `OwnStats`), next to `pump`, `counters`, and `with_links`. TRN-27 and TRN-32 can use the stack without the harness.
- `fps-tcp` keeps the harness, the `TcpStack` adapter of the own stack (`src/own.rs`), and the kernel and smoltcp adapters, and depends on `fps-tcp-own`. The paths `fps_tcp::own::{host, wire}`, `fps_tcp::clock`, and the types `OwnStack`, `OwnConfig`, `OwnCounters`, and `OwnSession` stay.
- The stack code did not change. The workload test of the own stack stays in `fps-tcp/tests/own_workloads.rs`.
- The tcp_bench run of the own stack after the move is in `engine/fps-tcp/results/sandbox-2vcpu-trn21/`, next to the earlier results.

The wiki page `Transport.md` names the new crate.

### Shared page notes

- Root `Cargo.toml`: no edit was needed. The workspace members glob `engine/*` already covers `engine/fps-tcp-own`. `Cargo.lock` changed (the new package, and `fps-tcp` depends on it instead of `fps-backend-afxdp`), so the pull request carries that file.
- Testing page: `cargo test -p fps-tcp-own` runs the component tests of the own stack, and `cargo test -p fps-tcp` the harness tests.
- CODEOWNERS and the Makefile name no path of `engine/fps-tcp`, so neither needs an entry for the new crate. If the coordinator keeps a list of the TRN crates anywhere, `fps-tcp-own` belongs on it.
