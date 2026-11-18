# fps-tcp

The TCP stack bench harness of the engine (TRN-17). It drives a TCP stack behind `SessionIo` and measures sessions per second, throughput, and scaling toward 1,000,000 sockets, so decision record 0014 of TRN-10 can compare stacks on one host. The kernel loopback TCP is the reference stack, TRN-18 added an own stack on the same trait, and TRN-20 added the smoltcp adapter (the license 0BSD of smoltcp and `managed` is allowed in `deny.toml` since 2026-11-17).

The full description, the results, and the limits are on the wiki page [Transport](../../docs/wiki/Transport.md#tcp-stack-bench-harness).

## Run it

```bash
cargo run --release -p fps-tcp --example tcp_bench -- --profile small
cargo run --release -p fps-tcp --example tcp_bench -- --stack own --profile small
cargo run --release -p fps-tcp --example tcp_bench -- --stack smoltcp --profile small
cargo run --release -p fps-tcp --example tcp_bench -- --profile full --label fat-nuc --out -
cargo run --release -p fps-tcp --example tcp_bench -- --help
```

The command takes plain arguments, needs no root, and writes an `fps-bench/v1` document with `--out PATH`, or to standard output with `--out -`. The committed results are in `results/`, each directory with a `host.txt` that names the host and the command. The committed results are in `results/`: `sandbox-2vcpu/` is the kernel reference of TRN-17, and `sandbox-2vcpu-trn18/` has the own stack and the kernel reference run again next to it, and `sandbox-2vcpu-trn20/` has smoltcp with the kernel and own stacks run again in one session. They come from the agent sandbox, which is not the GitHub runner and not fat-nuc. The scaling run of a kernel stack needs one file descriptor per socket, so it ends at the open files limit of the process, which the report lists as `open_files_soft_limit`.

## Layout

| Path | Contents |
|---|---|
| `src/stack.rs` | The `TcpStack` seam that every stack implements |
| `src/kernel.rs` | `KernelStack` and `KernelSession`, the kernel loopback reference stack |
| `src/own.rs` | The `TcpStack` adapter of the own stack. The stack itself (TRN-18) lives in the crate `engine/fps-tcp-own` since TRN-21, which has no smoltcp dependency: `src/host.rs` (state machine, timers), `src/wire.rs` (IPv4 and TCP frames), `src/lib.rs` (link, driver, `SessionIo`), and `src/clock.rs` |
| `src/smol.rs`, `src/link.rs` | `SmolStack` and `SmolSession` (TRN-20): the smoltcp adapter, and the in memory IP link that joins its two hosts |
| `src/sysinfo.rs` | The facts about the host. The clocks that a stack reads its time from are in `fps-tcp-own` and re-exported as `fps_tcp::clock` |
| `src/workload.rs` | The three workloads |
| `src/report.rs` | The text report and the `fps-bench/v1` document |
| `src/cli.rs` | The flags of `tcp_bench` |
| `tests/` | Reference stack tests through `SessionIo` (`kernel.rs`), workload tests (`workloads.rs`), the workloads on the own stack (`own_workloads.rs`; the component tests of the stack over a lossy link are in `fps-tcp-own/tests/own.rs`), and smoltcp adapter tests (`smol.rs`) |

## Add a stack

Implement `TcpStack` for a type that is cheap to clone, register its name in `cli.rs`, and run `tcp_bench --stack NAME`. The workloads need nothing else. The stack lists the facts that a reader needs for its numbers in `TcpStack::notes`. `SmolStack` (TRN-20) and `OwnStack` (TRN-18) are examples. The own stack runs over `fps_backend_afxdp::FrameIo`, so TRN-12 can run it over an AF_XDP queue with `OwnStack::with_links`. What it leaves out is listed in the module documentation of `engine/fps-tcp-own/src/lib.rs` and on the wiki page.
