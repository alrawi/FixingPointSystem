# fps-tcp

The TCP stack bench harness of the engine (TRN-17). It drives a user space TCP stack behind `SessionIo` and measures sessions per second, throughput, and scaling toward 1,000,000 sockets, so decision record 0014 of TRN-10 can compare stacks on one host. smoltcp is the first stack, and TRN-18 adds an own stack on the same trait.

The full description, the results, and the limits are on the wiki page [Transport](../../docs/wiki/Transport.md#tcp-stack-bench-harness).

## Run it

```bash
cargo run --release -p fps-tcp --example tcp_bench -- --profile small
cargo run --release -p fps-tcp --example tcp_bench -- --profile full --label fat-nuc --out -
cargo run --release -p fps-tcp --example tcp_bench -- --help
```

The command takes plain arguments, needs no root, and writes an `fps-bench/v1` document with `--out PATH`, or to standard output with `--out -`. The committed results are in `results/`, each directory with a `host.txt` that names the host and the command.

## Layout

| Path | Contents |
|---|---|
| `src/stack.rs` | The `TcpStack` seam that every stack implements |
| `src/smol.rs` | `SmolStack` and `SmolSession`, the smoltcp adapter |
| `src/link.rs` | The in memory link between the two hosts of a pair |
| `src/workload.rs` | The three workloads |
| `src/report.rs` | The text report and the `fps-bench/v1` document |
| `src/cli.rs` | The flags of `tcp_bench` |
| `tests/` | Adapter tests through `SessionIo` and workload tests |

## Add a stack

Implement `TcpStack` for a type that is cheap to clone, register its name in `cli.rs`, and run `tcp_bench --stack NAME`. The workloads need nothing else.
