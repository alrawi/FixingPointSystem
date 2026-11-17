# fps-tcp

The TCP stack bench harness of the engine (TRN-17). It drives a TCP stack behind `SessionIo` and measures sessions per second, throughput, and scaling toward 1,000,000 sockets, so decision record 0014 of TRN-10 can compare stacks on one host. The kernel loopback TCP is the reference stack. The smoltcp adapter is TRN-20, because smoltcp and `managed` are licensed 0BSD, which `deny.toml` does not allow yet, and its draft lives on the branch `wp/TRN-20` (1,000,584 sockets at 1.59 KB per socket, idle poll 15 to 52 ns per socket). TRN-18 added an own stack on the same trait, see below.

The full description, the results, and the limits are on the wiki page [Transport](../../docs/wiki/Transport.md#tcp-stack-bench-harness).

## Run it

```bash
cargo run --release -p fps-tcp --example tcp_bench -- --profile small
cargo run --release -p fps-tcp --example tcp_bench -- --stack own --profile small
cargo run --release -p fps-tcp --example tcp_bench -- --profile full --label fat-nuc --out -
cargo run --release -p fps-tcp --example tcp_bench -- --help
```

The command takes plain arguments, needs no root, and writes an `fps-bench/v1` document with `--out PATH`, or to standard output with `--out -`. The committed results are in `results/`, each directory with a `host.txt` that names the host and the command. The committed results are in `results/`: `sandbox-2vcpu/` is the kernel reference of TRN-17, and `sandbox-2vcpu-trn18/` has the own stack and the kernel reference run again next to it. They come from the agent sandbox, which is not the GitHub runner and not fat-nuc. The scaling run of a kernel stack needs one file descriptor per socket, so it ends at the open files limit of the process, which the report lists as `open_files_soft_limit`.

## Layout

| Path | Contents |
|---|---|
| `src/stack.rs` | The `TcpStack` seam that every stack implements |
| `src/kernel.rs` | `KernelStack` and `KernelSession`, the kernel loopback reference stack |
| `src/own/` | `OwnStack` and `OwnSession` (TRN-18): the minimal own TCP stack, with `host.rs` (state machine, timers), `wire.rs` (IPv4 and TCP frames), and `mod.rs` (link, driver, `SessionIo`) |
| `src/clock.rs`, `src/sysinfo.rs` | The clock that a stack reads its time from, and the facts about the host |
| `src/workload.rs` | The three workloads |
| `src/report.rs` | The text report and the `fps-bench/v1` document |
| `src/cli.rs` | The flags of `tcp_bench` |
| `tests/` | Reference stack tests through `SessionIo` (`kernel.rs`), workload tests (`workloads.rs`), and own stack tests over a lossy link (`own.rs`) |

## Add a stack

Implement `TcpStack` for a type that is cheap to clone, register its name in `cli.rs`, and run `tcp_bench --stack NAME`. The workloads need nothing else. The stack lists the facts that a reader needs for its numbers in `TcpStack::notes`. TRN-20 adds `SmolStack` this way, and `OwnStack` (TRN-18) is the example. The own stack runs over `fps_backend_afxdp::FrameIo`, so TRN-12 can run it over an AF_XDP queue with `OwnStack::with_links`. What it leaves out is listed in the module documentation of `src/own/mod.rs` and on the wiki page.
